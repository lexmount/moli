use super::source_phase::ModuleSourceServer;
use super::*;

#[tokio::test]
async fn worker_module_map_shares_fetches_across_graphs_and_parent_edges() {
    ensure_v8();
    let mut server = ModuleSourceServer::start().await;
    let mut handle = server.worker(
        r#"
        self.sharedEvaluations = 0;
        self.leafEvaluations = 0;
        const [a, b] = await Promise.all([import('./a.mjs'), import('./b.mjs')]);
        postMessage([a.value, b.value, self.sharedEvaluations, self.leafEvaluations,
            a === await import('./a.mjs'), b === await import('./b.mjs')]);
        "#
        .into(),
        WorkerScriptKind::Module,
    );
    let (a, b) = server.request_pair("/worker/a.mjs", "/worker/b.mjs").await;
    ModuleSourceServer::write(
        a,
        "200 OK",
        "text/javascript",
        r#"
        import { value as first } from './shared.mjs';
        import { value as second } from './dir/../shared.mjs';
        import './a-ready.mjs';
        export const value = first + second;
        "#,
    )
    .await;
    // Hold the shared response while both graphs subscribe to it. Distinct
    // sibling requests prove each graph has advanced through its own imports.
    let (shared, a_ready) = server
        .request_pair("/worker/shared.mjs", "/worker/a-ready.mjs")
        .await;
    ModuleSourceServer::write(
        b,
        "200 OK",
        "text/javascript",
        "export { value } from './shared.mjs'; import './b-ready.mjs';",
    )
    .await;
    let b_ready = server.request("/worker/b-ready.mjs").await;
    ModuleSourceServer::write(a_ready, "200 OK", "text/javascript", "").await;
    ModuleSourceServer::write(b_ready, "200 OK", "text/javascript", "").await;
    ModuleSourceServer::write(
        shared,
        "200 OK",
        "text/javascript",
        "export { value } from './leaf.mjs'; ++self.sharedEvaluations;",
    )
    .await;
    server
        .respond(
            "/worker/leaf.mjs",
            "200 OK",
            "text/javascript",
            "++self.leafEvaluations; export const value = 42;",
        )
        .await;
    assert_eq!(recv_post_json(&mut handle).await, "[84,42,1,1,true,true]");
    handle.terminate_and_join();
    server.assert_no_more_requests();
}

#[tokio::test]
async fn worker_module_map_compiled_root_still_waits_for_shared_evaluation() {
    ensure_v8();
    let mut server = ModuleSourceServer::start().await;
    let mut handle = server.worker(
        r#"
        (async () => {
        self.evaluations = 0;
        let resolutions = 0;
        let second;
        const first = import('./await.mjs').then(ns => { ++resolutions; return ns; });
        onmessage = event => {
            if (event.data === 'join') {
                second = import('./await.mjs').then(ns => { ++resolutions; return ns; });
                postMessage('joined');
            } else if (event.data === 'probe') {
                postMessage(['pending', resolutions]);
            } else if (event.data === 'release') {
                self.releaseEvaluation();
            }
        };
        const namespace = await first;
        postMessage([namespace === await second, self.evaluations, resolutions]);
        })().catch(error => postMessage({ error: error.name }));
        "#
        .into(),
        WorkerScriptKind::Module,
    );
    server
        .respond(
            "/worker/await.mjs",
            "200 OK",
            "text/javascript",
            r#"
            ++self.evaluations;
            await new Promise(resolve => {
                self.releaseEvaluation = resolve;
                postMessage('evaluating');
            });
            export const value = 42;
            "#,
        )
        .await;
    assert_eq!(recv_post_json(&mut handle).await, r#""evaluating""#);
    handle.post_message(serialize_test_string("join"));
    assert_eq!(recv_post_json(&mut handle).await, r#""joined""#);
    // A separate message runs after the import queue has drained, so an early
    // resolution cannot hide behind the acknowledgement's microtask ordering.
    handle.post_message(serialize_test_string("probe"));
    assert_eq!(recv_post_json(&mut handle).await, r#"["pending",0]"#);
    handle.post_message(serialize_test_string("release"));
    assert_eq!(recv_post_json(&mut handle).await, "[true,1,2]");
    handle.terminate_and_join();
    server.assert_no_more_requests();
}

#[tokio::test]
async fn worker_module_load_failure_preserves_pending_top_level_await() {
    module_load_failure_keeps_pending_sibling(true, false).await;
}

#[tokio::test]
async fn worker_module_load_failure_reuses_pending_and_completed_sibling() {
    module_load_failure_keeps_pending_sibling(false, false).await;
}

#[tokio::test]
async fn worker_module_load_failure_caches_sibling_after_graph_is_gone() {
    for top_level_await in [true, false] {
        module_load_failure_keeps_pending_sibling(top_level_await, true).await;
    }
}

async fn module_load_failure_keeps_pending_sibling(
    top_level_await: bool,
    sibling_response_first: bool,
) {
    ensure_v8();
    for imports in [
        r#"import './side.mjs'; import data from './bad.json' with {type:'json'};"#,
        r#"import data from './bad.json' with {type:'json'}; import './side.mjs';"#,
    ] {
        let mut server = ModuleSourceServer::start().await;
        let source = r#"
            self.runs = 0;
            let graphError;
            try { await import('./graph.mjs'); }
            catch (error) { graphError = error; }
            postMessage(['caught', graphError.name, self.runs]);
            await fetch('./hold');
            const pendingSide = import('./side.mjs');
            postMessage('joining sibling');
            try {
                const side = await pendingSide;
                const again = await import('./side.mjs');
                postMessage([true, graphError.name, side.value, self.runs,
                    side === again]);
            } catch (error) { postMessage({laterError: error.name}); }
        "#;
        let source = if top_level_await {
            source.to_owned()
        } else {
            format!(
                "(async () => {{ {source} }})().catch(error => postMessage({{error: error.name}}));"
            )
        };
        let mut handle = server.worker(source, WorkerScriptKind::Module);
        server
            .respond("/worker/graph.mjs", "200 OK", "text/javascript", imports)
            .await;
        let (sibling, invalid_json) = server
            .request_pair("/worker/side.mjs", "/worker/bad.json")
            .await;
        ModuleSourceServer::write(
            invalid_json,
            "503 Service Unavailable",
            "text/plain",
            "unavailable",
        )
        .await;
        assert_eq!(
            recv_post_json(&mut handle).await,
            r#"["caught","TypeError",0]"#
        );
        let start_independent_import = async {
            server
                .respond("/worker/hold", "200 OK", "text/plain", "release")
                .await;
            assert_eq!(recv_post_json(&mut handle).await, r#""joining sibling""#);
        };
        let sibling_response = ModuleSourceServer::write(
            sibling,
            "200 OK",
            "text/javascript",
            "++self.runs; export const value = 42;",
        );
        if sibling_response_first {
            // No live import needs this response yet; retain its module record.
            sibling_response.await;
            start_independent_import.await;
        } else {
            // The independent import must reuse the original in-flight fetch.
            start_independent_import.await;
            sibling_response.await;
        }
        let observed = tokio::select! {
            observed = recv_post_json(&mut handle) => observed,
            duplicate = server.request("/worker/side.mjs") => {
                ModuleSourceServer::write(duplicate, "503 Service Unavailable", "text/plain", "duplicate fetch").await;
                recv_post_json(&mut handle).await
            }
        };
        assert_eq!(observed, r#"[true,"TypeError",42,1,true]"#, "{imports}");
        handle.terminate_and_join();
        server.assert_no_more_requests();
    }
}

#[tokio::test]
async fn worker_module_tree_failure_preserves_admitted_network_actions() {
    ensure_v8();
    let mut server = ModuleSourceServer::start().await;
    let mut handle = server.worker(
        r#"
        (async () => {
            await import('./a.mjs').catch(() => {});
            await import('./b.mjs').catch(() => {});
            const failure = await import('./both.mjs').catch(error => error.name);
            postMessage(['failed', failure]);
            const retry = import('./a.mjs');
            postMessage('joined');
            postMessage((await retry).value);
        })().catch(error => postMessage({ error: error.name }));
    "#
        .into(),
        WorkerScriptKind::Module,
    );
    server
        .respond(
            "/worker/a.mjs",
            "200 OK",
            "text/javascript",
            "export { value } from './held.mjs';",
        )
        .await;
    server
        .respond(
            "/worker/held.mjs",
            "503 Service Unavailable",
            "text/plain",
            "retry later",
        )
        .await;
    server
        .respond(
            "/worker/b.mjs",
            "200 OK",
            "text/javascript",
            "import 'unresolvable-bare-specifier';",
        )
        .await;
    // Both records now exist. The common tree admits A's retry before B's
    // resolution failure ends the graph in the same drive call.
    server
        .respond(
            "/worker/both.mjs",
            "200 OK",
            "text/javascript",
            "import './a.mjs'; import './b.mjs';",
        )
        .await;
    let held = server.request("/worker/held.mjs").await;
    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"["failed","TypeError"]"#
    );
    assert_eq!(recv_post_json(&mut handle).await, r#""joined""#);
    ModuleSourceServer::write(
        held,
        "200 OK",
        "text/javascript",
        "export const value = 42;",
    )
    .await;
    assert_eq!(recv_post_json(&mut handle).await, "42");
    handle.terminate_and_join();
    server.assert_no_more_requests();
}

#[tokio::test]
async fn worker_shared_module_evaluation_preserves_original_rejection_value() {
    ensure_v8();
    let mut server = ModuleSourceServer::start().await;
    let mut handle = server.worker(r#"
        (async () => {
            self.evaluations = 0;
            self.coercions = 0;
            self.reason = { toString() { ++self.coercions; return 'coerced'; } };
            const capture = promise => promise.catch(error => error);
            const [first, second] = await Promise.all([capture(import('./async.mjs')), capture(import('./async.mjs'))]);
            const again = await capture(import('./async.mjs'));
            postMessage([first === self.reason, second === self.reason, again === self.reason,
                self.evaluations, self.coercions]);
        })().catch(error => postMessage({ error: error.name }));
    "#.into(), WorkerScriptKind::Module);
    server
        .respond(
            "/worker/async.mjs",
            "200 OK",
            "text/javascript",
            "++self.evaluations; await fetch('./eval-hold'); throw self.reason;",
        )
        .await;
    server
        .respond("/worker/eval-hold", "200 OK", "text/plain", "release")
        .await;
    assert_eq!(recv_post_json(&mut handle).await, "[true,true,true,1,0]");
    handle.terminate_and_join();
    server.assert_no_more_requests();
}

#[tokio::test]
async fn worker_distinct_roots_and_direct_import_share_pending_dependency_evaluation() {
    ensure_v8();
    let mut server = ModuleSourceServer::start().await;
    let mut handle = server.worker(
        r#"
        (async () => {
            self.evaluations = 0;
            let others;
            let resolutions = 0;
            onmessage = event => {
                if (event.data === 'join') {
                    others = Promise.all([import('./b.mjs'), import('./dep.mjs')].map(promise =>
                        promise.then(ns => { ++resolutions; return ns; })));
                    postMessage('joined');
                } else if (event.data === 'probe') {
                    postMessage(['pending', resolutions]);
                }
            };
            const first = await import('./a.mjs');
            const [second, direct] = await others;
            postMessage([first.value, second.value, direct.value, self.evaluations,
                direct === await import('./dep.mjs')]);
        })().catch(error => postMessage({ error: error.name }));
    "#
        .into(),
        WorkerScriptKind::Module,
    );
    server
        .respond(
            "/worker/a.mjs",
            "200 OK",
            "text/javascript",
            "export { value } from './dep.mjs';",
        )
        .await;
    server
        .respond(
            "/worker/dep.mjs",
            "200 OK",
            "text/javascript",
            "++self.evaluations; await fetch('./eval-hold'); export const value = 42;",
        )
        .await;
    let hold = server.request("/worker/eval-hold").await;
    handle.post_message(serialize_test_string("join"));
    assert_eq!(recv_post_json(&mut handle).await, r#""joined""#);
    server
        .respond(
            "/worker/b.mjs",
            "200 OK",
            "text/javascript",
            "export { value } from './dep.mjs';",
        )
        .await;
    handle.post_message(serialize_test_string("probe"));
    assert_eq!(recv_post_json(&mut handle).await, r#"["pending",0]"#);
    ModuleSourceServer::write(hold, "200 OK", "text/plain", "release").await;
    assert_eq!(recv_post_json(&mut handle).await, "[42,42,42,1,true]");
    handle.terminate_and_join();
    server.assert_no_more_requests();
}

#[tokio::test]
async fn worker_close_releases_pending_module_rejection_handles() {
    ensure_v8();
    let mut server = ModuleSourceServer::start().await;
    let mut handle = server.worker(
        r#"
        import('./close.mjs').catch(() => postMessage('unexpected callback after close'));
    "#
        .into(),
        WorkerScriptKind::Module,
    );
    server
        .respond(
            "/worker/close.mjs",
            "200 OK",
            "text/javascript",
            r#"
        await fetch('./close-hold');
        self.close();
        throw { toString() { throw new Error('unexpected coercion after close'); } };
    "#,
        )
        .await;
    server
        .respond("/worker/close-hold", "200 OK", "text/plain", "release")
        .await;
    while let Some(message) = timeout(TIMEOUT, handle.recv())
        .await
        .expect("closed worker should exit")
    {
        match message {
            WorkerToParentMessage::Post(_) | WorkerToParentMessage::Error { .. } => {
                panic!("unexpected script activity during module retirement: {message:?}")
            }
            _ => {}
        }
    }
    handle.terminate_and_join();
    server.assert_no_more_requests();
}

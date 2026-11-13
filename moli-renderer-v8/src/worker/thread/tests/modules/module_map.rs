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

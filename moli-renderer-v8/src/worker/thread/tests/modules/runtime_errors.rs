use super::source_phase::ModuleSourceServer;
use super::*;
use crate::worker::WorkerErrorSource;

#[tokio::test]
async fn worker_module_evaluation_failures_keep_queued_work_alive() {
    ensure_v8();
    for imported in [false, true] {
        for handled in [false, true] {
            for failure in [
                "throw marker;",
                "await Promise.reject(marker);",
                "await new Promise((_, reject) => setTimeout(() => reject(marker), 0));",
            ] {
                let mut server = ModuleSourceServer::start().await;
                let source = format!(
                    r#"
                    const marker = new Error('initial module error');
                    const log = [];
                    let unhandled = 0;
                    let sameReason = false;
                    addEventListener('unhandledrejection', event => {{
                        ++unhandled;
                        event.preventDefault();
                    }});
                    function send(part) {{ postMessage({{part, log, sameReason, unhandled}}); }}
                    addEventListener('error', event => {{
                        sameReason = event.error === marker;
                        log.push('error-1');
                        Promise.resolve().then(() => log.push('microtask-1'));
                        if ({handled}) event.preventDefault();
                        setTimeout(() => send('timer'), 0);
                    }});
                    addEventListener('error', () => {{
                        log.push('error-2');
                        Promise.resolve().then(() => log.push('microtask-2'));
                    }});
                    onmessage = () => send('message');
                    {failure}
                    "#
                );
                let mut worker = server.worker(
                    if imported {
                        "import './throw.mjs';".into()
                    } else {
                        source.clone()
                    },
                    WorkerScriptKind::Module,
                );
                worker.post_message(serialize_test_string("queued before evaluation"));
                if imported {
                    server
                        .respond("/worker/throw.mjs", "200 OK", "text/javascript", &source)
                        .await;
                }
                let mut parts = Vec::new();
                let mut parent_errors = 0;
                while parts.len() < 2 || (!handled && parent_errors == 0) {
                    match timeout(TIMEOUT, worker.recv()).await.unwrap().unwrap() {
                        WorkerToParentMessage::Post(payload) => {
                            let value: serde_json::Value =
                                serde_json::from_str(&stringify_payload(&payload)).unwrap();
                            assert_eq!(
                                value["log"],
                                serde_json::json!([
                                    "error-1",
                                    "error-2",
                                    "microtask-1",
                                    "microtask-2"
                                ])
                            );
                            assert_eq!(value["sameReason"], true);
                            assert_eq!(value["unhandled"], 0);
                            parts.push(value["part"].as_str().unwrap().to_owned());
                        }
                        WorkerToParentMessage::Error {
                            message,
                            event_kind,
                            phase,
                            source,
                            ..
                        } => {
                            assert!(!handled, "canceled error reached parent: {message}");
                            assert!(message.contains("initial module error"), "{message}");
                            assert_eq!(event_kind, WorkerParentErrorEventKind::ErrorEvent);
                            assert_eq!(phase, WorkerErrorPhase::Runtime);
                            assert_eq!(source, WorkerErrorSource::InitialScriptEvaluation);
                            parent_errors += 1;
                        }
                        other => panic!("unexpected worker event: {other:?}"),
                    }
                }
                parts.sort();
                assert_eq!(parts, ["message", "timer"]);
                assert_eq!(parent_errors, usize::from(!handled));
                worker.terminate_and_join();
            }
        }
    }
}

#[tokio::test]
async fn worker_module_await_rejections_preserve_values_without_coercion() {
    ensure_v8();
    for reason in [
        "new TypeError('original error')",
        "({[Symbol.toPrimitive]() { ++reads; throw new Error('author conversion'); }})",
        "undefined",
        "Symbol('original symbol')",
        "17",
    ] {
        let mut worker = spawn_worker_with_request_client_and_kind(
            format!(
                r#"
                let reads = 0;
                Error.prepareStackTrace = () => {{ ++reads; throw new Error('author stack'); }};
                const marker = {reason};
                addEventListener('error', event => {{
                    const same = event.error === marker;
                    event.preventDefault();
                    setTimeout(() => postMessage([same, reads]), 0);
                }});
                await new Promise((_, reject) => setTimeout(() => reject(marker), 0));
            "#
            ),
            "https://worker.test/main.mjs".into(),
            worker_test_request_client(),
            WorkerScriptKind::Module,
        );
        assert_eq!(recv_post_json(&mut worker).await, "[true,0]", "{reason}");
        worker.terminate_and_join();
    }
}

#[tokio::test]
async fn worker_dynamic_module_await_rejections_keep_cached_identity() {
    ensure_v8();
    for kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
        let mut server = ModuleSourceServer::start().await;
        let mut worker = server.worker(r#"
            let reads = 0;
            let unhandled = 0;
            self.marker = {[Symbol.toPrimitive]() { ++reads; throw new Error('author conversion'); }};
            addEventListener('unhandledrejection', event => { ++unhandled; event.preventDefault(); });
            const capture = () => import('./reject.mjs').catch(error => error);
            Promise.all([capture(), capture()]).then(async errors => {
                const again = await capture();
                setTimeout(() => postMessage([errors[0] === marker,
                    errors[0] === errors[1], errors[0] === again, reads, unhandled]), 0);
            });
        "#.into(), kind);
        server
            .respond(
                "/worker/reject.mjs",
                "200 OK",
                "text/javascript",
                "await new Promise((_, reject) => setTimeout(() => reject(marker), 0));",
            )
            .await;
        assert_eq!(recv_post_json(&mut worker).await, "[true,true,true,0,0]");
        worker.terminate_and_join();
        server.assert_no_more_requests();
    }
}

#[tokio::test]
async fn service_worker_module_evaluation_failure_still_rejects_startup() {
    ensure_v8();
    for imported in [false, true] {
        let mut server = ModuleSourceServer::start().await;
        let source = "setTimeout(() => console.log('unexpected timer'), 0); throw new Error('service evaluation failed');";
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let mut worker = spawn_test_worker_with_options(
            service_worker_module_options(
                if imported {
                    "import './throw.mjs';"
                } else {
                    source
                }
                .into(),
                format!("{}/worker/main.mjs", server.url),
            )
            .with_bootstrap_completion_sender(sender),
        );
        if imported {
            server
                .respond("/worker/throw.mjs", "200 OK", "text/javascript", source)
                .await;
        }
        let completion = timeout(TIMEOUT, receiver.recv()).await.unwrap().unwrap();
        let failure = completion.result.unwrap_err();
        assert!(failure.message.contains("service evaluation failed"));
        assert_eq!(failure.phase, WorkerErrorPhase::Runtime);
        let mut errors = 0;
        while let Some(message) = timeout(TIMEOUT, worker.recv()).await.unwrap() {
            match message {
                WorkerToParentMessage::ServiceWorkerImportedScriptLoaded { .. } => {}
                WorkerToParentMessage::Error { .. } => errors += 1,
                other => panic!("failed service worker executed a task: {other:?}"),
            }
        }
        assert_eq!(errors, 1);
        worker.terminate_and_join();
    }
}

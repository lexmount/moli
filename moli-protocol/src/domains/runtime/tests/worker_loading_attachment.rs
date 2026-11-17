use super::*;
use axum::http::StatusCode;
use serde_json::Value;
use std::{future::Future, sync::Arc, time::Duration};
use tokio::sync::Notify;

async fn command(
    ctx: &mut TestContext,
    id: u64,
    session: Option<&str>,
    method: &str,
    params: Value,
) -> Value {
    let mut request = json!({"id":id, "method":method, "params":params});
    if let Some(session) = session {
        request["sessionId"] = json!(session);
    }
    ctx.process_and_wait_for_response_async(request).await;
    let response = take_response_by_id(ctx, id);
    assert!(response.get("error").is_none(), "{response}");
    assert!(
        response["result"].get("exceptionDetails").is_none(),
        "{response}"
    );
    response["result"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn dedicated_worker_early_attachment_survives_script_loading() {
    check_loading_attachment(AttachmentScenario::AfterLoad).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dedicated_worker_loading_does_not_restore_detached_inspector_session() {
    check_loading_attachment(AttachmentScenario::Reattach).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dedicated_worker_inspector_command_waits_for_script_loading() {
    check_loading_attachment(AttachmentScenario::CommandBeforeLoad).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dedicated_worker_loading_detach_settles_queued_inspector_command() {
    check_loading_attachment(AttachmentScenario::DetachPendingCommand).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dedicated_worker_script_failure_settles_queued_inspector_command() {
    check_loading_attachment(AttachmentScenario::FailedLoad).await;
}

#[derive(Clone, Copy)]
enum AttachmentScenario {
    AfterLoad,
    Reattach,
    CommandBeforeLoad,
    DetachPendingCommand,
    FailedLoad,
}

async fn check_loading_attachment(scenario: AttachmentScenario) {
    let release = Arc::new(Notify::new());
    let requested = Arc::new(Notify::new());
    let worker_release = release.clone();
    let worker_requested = requested.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route(
            "/page",
            get(|| async {
                (
                    [(CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><title>worker attachment</title>",
                )
            }),
        )
        .route(
            "/worker.js",
            get(move || {
                let release = worker_release.clone();
                let requested = worker_requested.clone();
                async move {
                    requested.notify_one();
                    release.notified().await;
                    (
                        if matches!(scenario, AttachmentScenario::FailedLoad) {
                            StatusCode::SERVICE_UNAVAILABLE
                        } else {
                            StatusCode::OK
                        },
                        [(CONTENT_TYPE.as_str(), "text/javascript")],
                        "globalThis.attachmentProbe = 42; postMessage('ready');",
                    )
                }
            }),
        );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &format!("{base}/page"), "SID-page", "TID-page")
        .await;
    command(
        &mut ctx,
        80100,
        None,
        "Target.setDiscoverTargets",
        json!({"discover":true}),
    )
    .await;
    command(
        &mut ctx,
        80101,
        Some("SID-page"),
        "Runtime.enable",
        json!({}),
    )
    .await;
    command(&mut ctx, 80102, Some("SID-page"), "Runtime.evaluate", json!({
        "expression":"globalThis.workerReady = false; globalThis.worker = new Worker('/worker.js'); worker.onmessage = () => {workerReady = true;}; 'started'",
        "returnByValue":true
    })).await;
    tokio::time::timeout(Duration::from_secs(5), requested.notified())
        .await
        .unwrap();
    wait_until_message(
        &mut ctx,
        None,
        "loading DedicatedWorker target",
        |message| {
            message["method"] == "Target.targetCreated"
                && message["params"]["targetInfo"]["type"] == "worker"
                && message["params"]["targetInfo"]["openerId"] == "TID-page"
        },
    )
    .await;
    let target = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == "Target.targetCreated"
                && message["params"]["targetInfo"]["type"] == "worker"
                && message["params"]["targetInfo"]["openerId"] == "TID-page"
        })
        .unwrap()["params"]["targetInfo"]["targetId"]
        .as_str()
        .unwrap()
        .to_owned();
    let original_session = command(
        &mut ctx,
        80103,
        None,
        "Target.attachToTarget",
        json!({"targetId":target,"flatten":true}),
    )
    .await["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    let (runtime, instance) = ctx.with_fixture(|conn| {
        let context = conn.browser_context.as_ref().unwrap();
        (
            context.renderer_runtime(),
            context
                .dedicated_worker_target(&target)
                .unwrap()
                .renderer_instance_id,
        )
    });
    let pending_command = matches!(
        scenario,
        AttachmentScenario::CommandBeforeLoad
            | AttachmentScenario::DetachPendingCommand
            | AttachmentScenario::FailedLoad
    );
    // Admit renderer work without making TestContext synchronously await its
    // result. CDP still owns the real target attachment and its detachment.
    let mut queued_command = pending_command.then(|| Box::pin(runtime.dispatch_dedicated_worker_runtime_protocol_message(
        instance, Some(original_session.clone()),
        json!({"id":80106,"method":"Runtime.evaluate","params":{"expression":"21*2","returnByValue":true}}).to_string(),
    )));
    if let Some(pending) = queued_command.as_mut() {
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(pending.as_mut().poll(cx)))
                .await
                .is_pending()
        );
    }
    let detach_first = matches!(
        scenario,
        AttachmentScenario::Reattach | AttachmentScenario::DetachPendingCommand
    );
    let session = if detach_first {
        command(
            &mut ctx,
            80104,
            None,
            "Target.detachFromTarget",
            json!({"sessionId":original_session}),
        )
        .await;
        if pending_command {
            assert!(
                tokio::time::timeout(Duration::from_secs(5), queued_command.take().unwrap())
                    .await
                    .unwrap()
                    .is_err()
            );
        }
        command(
            &mut ctx,
            80105,
            None,
            "Target.attachToTarget",
            json!({"targetId":target,"flatten":true}),
        )
        .await["sessionId"]
            .as_str()
            .unwrap()
            .to_owned()
    } else {
        original_session.clone()
    };
    release.notify_one();
    if matches!(scenario, AttachmentScenario::FailedLoad) {
        ctx.wait_for_scheduler_message("failed Worker target retirement", |message| {
            message["method"] == "Target.targetDestroyed" && message["params"]["targetId"] == target
        })
        .await;
        assert!(
            tokio::time::timeout(Duration::from_secs(5), queued_command.take().unwrap())
                .await
                .unwrap()
                .is_err()
        );
        let value = command(
            &mut ctx,
            81002,
            Some("SID-page"),
            "Runtime.evaluate",
            json!({"expression":"42", "returnByValue":true}),
        )
        .await;
        assert_eq!(value["result"]["value"], 42);
        server.abort();
        return;
    }
    for index in 0..256 {
        let value = command(
            &mut ctx,
            80200 + index,
            Some("SID-page"),
            "Runtime.evaluate",
            json!({"expression":"workerReady","returnByValue":true}),
        )
        .await;
        if value["result"]["value"] == true {
            break;
        }
        assert!(index < 255, "worker script did not run");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    if detach_first {
        let late = runtime
            .dispatch_dedicated_worker_runtime_protocol_message(
                instance,
                Some(original_session),
                json!({"id":1,"method":"Runtime.enable"}).to_string(),
            )
            .await;
        assert!(
            late.is_err(),
            "detached early session must not be installed in the live renderer: {late:?}"
        );
    }
    if pending_command && !detach_first {
        let messages = tokio::time::timeout(Duration::from_secs(5), queued_command.take().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert!(messages.iter().any(|message| matches!(message,
            moli_core::page::RendererRuntimeInspectorMessage::Protocol(response)
            if response.value()["id"] == 80106 && response.value()["result"]["result"]["value"] == 42
        )), "{messages:?}");
    }
    ctx.sent.clear();
    command(&mut ctx, 81000, Some(&session), "Runtime.enable", json!({})).await;
    wait_until_message(
        &mut ctx,
        Some(session.as_str()),
        "early attachment execution context",
        |message| {
            message["method"] == "Runtime.executionContextCreated"
                && message["sessionId"] == session
        },
    )
    .await;
    let value = command(
        &mut ctx,
        81001,
        Some(&session),
        "Runtime.evaluate",
        json!({"expression":"attachmentProbe","returnByValue":true}),
    )
    .await;
    assert_eq!(value["result"]["value"], 42);
    server.abort();
}

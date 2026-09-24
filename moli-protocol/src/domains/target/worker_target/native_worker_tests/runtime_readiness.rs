use super::*;
use crate::domains::runtime::{
    CompletedRuntimeCommandDispatch, RuntimeCommandTaskStep,
    complete_pending_runtime_command_at_response_boundary, try_start_runtime_command_dispatch,
};
use moli_core::browser::{ServiceWorkerExecution, ServiceWorkerSnapshot};

struct PendingObserver {
    fixture: NativeWorkers,
    server: moli_test_support::FixtureServer,
    conn: CdpConnection,
    context_id: String,
    session_id: String,
    worker: ServiceWorkerSnapshot,
    outputs: TargetPreparedOutputs,
    delayed_enable: Option<CompletedRuntimeCommandDispatch>,
}

impl PendingObserver {
    async fn start() -> Self {
        Self::start_with_interleaved_enable(false).await
    }

    async fn start_with_interleaved_enable(interleave_enable: bool) -> Self {
        let server = moli_test_support::FixtureServer::spawn().await.unwrap();
        let fixture = NativeWorkers::start(&[]).await;
        let first = fixture
            .navigate_service_worker(&server.url("/native-service-worker/"))
            .await;
        let browser = fixture.service.handle();
        let mut conn = fixture.connection();
        conn.project_browser_snapshot(browser.subscribe().unwrap().0)
            .await;
        let context_id = conn
            .browser_context_by_browser_id(fixture.context.id())
            .unwrap()
            .id
            .clone();
        let target_id = conn
            .browser_context_by_id(&context_id)
            .unwrap()
            .service_worker_targets[&first.info.version_id]
            .target_id
            .clone();
        let (_, mut events) = browser.subscribe().unwrap();
        fixture
            .context
            .execute_service_worker_command(ServiceWorkerCommand::StopVersion {
                version_id: first.info.version_id,
            })
            .unwrap();
        wait_service_worker_state(&mut events, |worker| {
            worker.execution == ServiceWorkerExecution::Stopped
        })
        .await;
        conn.project_browser_snapshot(browser.subscribe().unwrap().0)
            .await;
        let (messages, work) = conn
            .enable_runtime_listener_for_target(&target_id)
            .await
            .unwrap()
            .into_parts();
        assert!(work.is_empty());
        assert!(
            messages
                .iter()
                .any(|message| message["result"] == json!({}))
        );
        let session_id = format!("SID-bidi-runtime-listener-service-worker-{target_id}");
        assert!(
            conn.service_worker_target_for_session(Some(&session_id))
                .unwrap()
                .runtime_frontend_enabled(&session_id)
        );
        let delayed_enable = if interleave_enable {
            conn.service_worker_target_for_session_mut(Some(&session_id))
                .unwrap()
                .set_runtime_frontend_enabled(&session_id, false);
            let params = json!({});
            let cmd = crate::conn::Cmd::for_test(
                Some(71),
                "Runtime.enable",
                &params,
                Some(&session_id),
                r#"{"id":71,"method":"Runtime.enable"}"#,
            );
            let Some(RuntimeCommandTaskStep::Pending(pending)) =
                try_start_runtime_command_dispatch(&mut conn, &cmd)
            else {
                panic!("capture the unavailable dispatch before starting the executor");
            };
            Some(pending.wait().await)
        } else {
            None
        };

        assert!(
            fixture
                .context
                .set_service_worker_pause_on_start_for_version(first.info.version_id, true)
        );
        fixture
            .context
            .execute_service_worker_command(ServiceWorkerCommand::Start {
                scope: first.info.scope_url.parse().unwrap(),
            })
            .unwrap();
        let worker = wait_service_worker_state(&mut events, |worker| {
            matches!(worker.execution, ServiceWorkerExecution::Bootstrapping(_))
        })
        .await;
        assert_ne!(worker.execution.active_run(), first.execution.active_run());
        // Recover the native readiness fact from a snapshot without draining
        // the source FIFO. The bootstrap cannot complete before debugger release.
        let snapshot = browser.subscribe().unwrap().0;
        assert!(snapshot.workers.iter().any(|snapshot| {
            matches!(snapshot, WorkerSnapshot::Service { worker: current, .. } if current == &worker)
        }));
        let outputs = project_service_worker_snapshot(&mut conn, &context_id, worker.clone());
        if interleave_enable {
            assert!(
                outputs.is_empty(),
                "readiness arrives before the delayed logical enable"
            );
        } else {
            assert!(matches!(
                outputs.worker_target_lifecycle_outputs.as_slice(),
                [WorkerTargetLifecycleOutput::RuntimeObserverReady(
                    WorkerRuntimeObserver::Service(_)
                )]
            ));
        }
        Self {
            fixture,
            server,
            conn,
            context_id,
            session_id,
            worker,
            outputs,
            delayed_enable,
        }
    }

    async fn shutdown(self) {
        self.fixture.service.shutdown();
        self.server.shutdown().await;
    }
}

#[tokio::test]
async fn native_service_worker_readiness_before_enable_completion_still_binds_observer() {
    let mut pending = PendingObserver::start_with_interleaved_enable(true).await;
    let completed = pending.delayed_enable.take().unwrap();
    let RuntimeCommandTaskStep::Complete(plan) =
        complete_pending_runtime_command_at_response_boundary(
            &mut pending.conn,
            completed,
            &crate::conn::CommandResponseFlushContext::default(),
        )
        .await
    else {
        panic!("logical enable and physical binding must complete together");
    };
    let mut messages = Vec::new();
    plan.emit_into(&mut messages, Some(71), Some(&pending.session_id));
    assert!(
        messages
            .iter()
            .any(|message| message["id"] == 71 && message["result"] == json!({})),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message["method"] == "Runtime.executionContextCreated"),
        "readiness already consumed before fallback must still install the real observer: {messages:?}"
    );
    pending.shutdown().await;
}

#[tokio::test]
async fn native_service_worker_runtime_listener_recovers_before_paused_bootstrap() {
    let mut pending = PendingObserver::start().await;
    let outputs = std::mem::take(&mut pending.outputs);
    let events = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        worker_target_background_events_async(&mut pending.conn, outputs),
    )
    .await
    .expect("Runtime observation must not wait for script completion or its own source FIFO");
    let messages = events
        .into_iter()
        .map(BackgroundProtocolEvent::into_protocol_message)
        .collect::<Vec<_>>();
    let created = messages
        .iter()
        .find(|message| message["method"] == "Runtime.executionContextCreated")
        .unwrap_or_else(|| {
            panic!("real Runtime context must precede debugger release: {messages:?}")
        });
    assert_eq!(created["sessionId"], pending.session_id);
    assert!(
        created["params"]["context"]["uniqueId"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
    assert!(
        messages.iter().all(|message| message.get("id").is_none()),
        "internal enable reply must not escape: {messages:?}"
    );
    assert!(
        !pending
            .conn
            .service_worker_target_for_session(Some(&pending.session_id))
            .unwrap()
            .worker_running()
    );
    let (_, mut native_events) = pending.fixture.service.handle().subscribe().unwrap();
    let release = pending
        .conn
        .start_worker_runtime_protocol_message_for_session(
            Some(&pending.session_id),
            r#"{"id":72,"method":"Runtime.runIfWaitingForDebugger"}"#.into(),
        )
        .unwrap();
    let release = release.wait().await.unwrap();
    let release = pending
        .conn
        .complete_worker_runtime_protocol_message_for_session(release)
        .unwrap();
    assert!(
        release.iter().any(
            |message| matches!(message, RendererRuntimeInspectorMessage::Protocol(message)
        if message.value()["id"] == 72 && message.value()["result"] == json!({}))
        ),
        "{release:?}"
    );
    wait_service_worker_state(&mut native_events, |worker| {
        matches!(worker.execution, ServiceWorkerExecution::Running(_))
    })
    .await;
    let events = pending
        .conn
        .project_browser_snapshot(pending.fixture.service.handle().subscribe().unwrap().0)
        .await;
    assert!(
        events
            .into_iter()
            .all(|event| event.into_protocol_message()["method"]
                != "Runtime.executionContextCreated"),
        "snapshot recovery and Started must not duplicate the same real realm"
    );
    pending.shutdown().await;
}

#[tokio::test]
async fn native_service_worker_readiness_cannot_reenable_a_disabled_observer() {
    let mut pending = PendingObserver::start().await;
    pending
        .conn
        .service_worker_target_for_session_mut(Some(&pending.session_id))
        .unwrap()
        .set_runtime_frontend_enabled(&pending.session_id, false);
    let outputs = std::mem::take(&mut pending.outputs);
    assert!(
        worker_target_background_events_async(&mut pending.conn, outputs)
            .await
            .is_empty()
    );
    assert!(
        pending
            .conn
            .service_worker_target_for_session(Some(&pending.session_id))
            .unwrap()
            .runtime_execution_context()
            .is_none()
    );
    pending.shutdown().await;
}

#[tokio::test]
async fn native_service_worker_readiness_cannot_bind_a_reused_session() {
    let mut pending = PendingObserver::start().await;
    let target = pending
        .conn
        .service_worker_target_for_session_mut(Some(&pending.session_id))
        .unwrap();
    assert!(target.detach_session(&pending.session_id));
    target.attach_session(pending.session_id.clone());
    target.set_runtime_frontend_enabled(&pending.session_id, true);
    let outputs = std::mem::take(&mut pending.outputs);
    assert!(
        worker_target_background_events_async(&mut pending.conn, outputs)
            .await
            .is_empty()
    );
    assert!(
        pending
            .conn
            .service_worker_target_for_session(Some(&pending.session_id))
            .unwrap()
            .runtime_execution_context()
            .is_none()
    );
    pending.shutdown().await;
}

#[tokio::test]
async fn native_service_worker_readiness_cannot_survive_context_close() {
    let mut pending = PendingObserver::start().await;
    assert!(pending.fixture.context.remove().unwrap());
    let outputs = std::mem::take(&mut pending.outputs);
    assert!(
        worker_target_background_events_async(&mut pending.conn, outputs)
            .await
            .is_empty()
    );
    assert!(
        pending
            .conn
            .service_worker_target_for_session(Some(&pending.session_id))
            .is_some(),
        "the closed Core must reject binding even before Protocol projects retirement"
    );
    pending
        .conn
        .project_browser_snapshot(pending.fixture.service.handle().subscribe().unwrap().0)
        .await;
    assert!(
        pending
            .conn
            .service_worker_target_for_session(Some(&pending.session_id))
            .is_none()
    );
    pending.shutdown().await;
}

#[tokio::test]
async fn native_service_worker_readiness_cannot_follow_a_retired_run() {
    let mut pending = PendingObserver::start().await;
    let browser = pending.fixture.service.handle();
    let (_, mut events) = browser.subscribe().unwrap();
    pending
        .fixture
        .context
        .execute_service_worker_command(ServiceWorkerCommand::StopVersion {
            version_id: pending.worker.info.version_id,
        })
        .unwrap();
    wait_service_worker_state(&mut events, |worker| {
        worker.execution == ServiceWorkerExecution::Stopped
    })
    .await;
    pending
        .conn
        .project_browser_snapshot(browser.subscribe().unwrap().0)
        .await;
    pending
        .fixture
        .context
        .execute_service_worker_command(ServiceWorkerCommand::Start {
            scope: pending.worker.info.scope_url.parse().unwrap(),
        })
        .unwrap();
    let restarted = wait_service_worker_state(&mut events, |worker| {
        matches!(worker.execution, ServiceWorkerExecution::Bootstrapping(_))
    })
    .await;
    assert_ne!(
        restarted.execution.active_run(),
        pending.worker.execution.active_run()
    );
    let current =
        project_service_worker_snapshot(&mut pending.conn, &pending.context_id, restarted);
    let outputs = std::mem::take(&mut pending.outputs);
    assert!(
        worker_target_background_events_async(&mut pending.conn, outputs)
            .await
            .is_empty()
    );
    assert!(
        worker_target_background_events_async(&mut pending.conn, current)
            .await
            .into_iter()
            .any(|event| event.into_protocol_message()["method"]
                == "Runtime.executionContextCreated")
    );
    pending.shutdown().await;
}

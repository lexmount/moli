use super::*;
use crate::domains::runtime::{
    RuntimeCommandTaskStep, complete_pending_runtime_command_at_response_boundary,
    try_start_runtime_command_dispatch,
};
use moli_core::page::RendererWorkerIdentity;
use std::sync::Arc;

struct LoadingObserver {
    fixture: NativeWorkers,
    conn: CdpConnection,
    context_id: String,
    instance: u64,
    session_id: String,
    release: Arc<tokio::sync::Semaphore>,
    server: tokio::task::JoinHandle<()>,
}

impl LoadingObserver {
    async fn start() -> Self {
        let fixture = NativeWorkers::start(&[]).await;
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let gate = release.clone();
        let app = axum::Router::new()
            .route("/", axum::routing::get(|| async {
                axum::response::Html("<script>globalThis.worker = new Worker('/worker.js', {name:'loading-observer'});</script>")
            }))
            .route("/worker.js", axum::routing::get(move || {
                let gate = gate.clone();
                async move {
                    gate.acquire().await.unwrap().forget();
                    ([("content-type", "text/javascript")], "globalThis.bootstrapRan = true; console.log('dedicated-readiness-start'); onmessage = () => {};")
                }
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let browser = fixture.service.handle();
        let (snapshot, mut events) = browser.subscribe().unwrap();
        let contents = snapshot.web_contents[0];
        let navigation = fixture
            .context
            .navigate_document(
                contents,
                moli_core::browser::web_contents::NavigationRequestInterception::new(
                    url.parse().unwrap(),
                    "GET".into(),
                    None,
                    Vec::new(),
                    NavigationRequestLoadPolicy::BrowserInitiated,
                ),
            )
            .unwrap();
        assert!(matches!(
            navigation.wait().await.unwrap(),
            moli_core::browser::BrowserNavigationOutcome::Document(_)
        ));
        let instance = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let BrowserEvent::WorkerCreated(WorkerSnapshot::Dedicated { worker, .. }) =
                    events.recv().await.unwrap().event
                {
                    assert!(worker.main_script.is_none());
                    break worker.info.instance_id;
                }
            }
        })
        .await
        .expect("creation precedes the gated HTTP script response");
        assert!(
            fixture
                .context
                .worker_inspection_endpoint(RendererWorkerIdentity::Dedicated(instance))
                .is_none()
        );
        let mut conn = fixture.connection();
        conn.project_browser_snapshot(browser.subscribe().unwrap().0)
            .await;
        let context = conn
            .browser_context_by_browser_id(fixture.context.id())
            .unwrap();
        let context_id = context.id.clone();
        let target_id = context.dedicated_worker_targets[&instance]
            .target_id
            .clone();
        let (messages, work) = conn
            .enable_runtime_listener_for_target(&target_id)
            .await
            .unwrap()
            .into_parts();
        assert!(work.is_empty());
        assert!(
            messages
                .iter()
                .any(|message| message["result"] == json!({})),
            "{messages:?}"
        );
        assert!(
            messages
                .iter()
                .all(|message| message["method"] != "Runtime.executionContextCreated")
        );
        let session_id = format!("SID-bidi-runtime-listener-dedicated-worker-{target_id}");
        // Context projection installs its observer defaults. Set the physical
        // pause after that projection, still before releasing the HTTP script.
        fixture.context.set_dedicated_worker_pause_on_start(true);
        assert!(
            conn.shared_worker_target_for_session(Some(&session_id))
                .unwrap()
                .runtime_frontend_enabled(&session_id)
        );
        Self {
            fixture,
            conn,
            context_id,
            instance,
            session_id,
            release,
            server,
        }
    }

    async fn loaded(&mut self) -> TargetPreparedOutputs {
        let (_, mut events) = self.fixture.service.handle().subscribe().unwrap();
        self.release.add_permits(1);
        let script = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let BrowserEvent::WorkerUpdated(WorkerSnapshot::Dedicated { worker, .. }) =
                    events.recv().await.unwrap().event
                    && worker.info.instance_id == self.instance
                    && let Some(script) = worker.main_script
                {
                    assert!(matches!(
                        script.outcome,
                        RendererDedicatedWorkerMainScriptOutcome::Loaded(_)
                    ));
                    break script;
                }
            }
        })
        .await
        .expect("native main-script completion needs no Protocol consumer");
        assert!(
            self.fixture
                .context
                .worker_inspection_endpoint(RendererWorkerIdentity::Dedicated(self.instance))
                .is_some()
        );
        project_dedicated_worker_main_script(
            &mut self.conn,
            &self.context_id,
            self.instance,
            script,
        )
    }

    fn target_mut(&mut self) -> &mut crate::conn::DedicatedWorkerTargetState {
        self.conn
            .browser_context_by_id_mut(&self.context_id)
            .unwrap()
            .dedicated_worker_targets
            .get_mut(&self.instance)
            .unwrap()
    }

    fn reuse_session(&mut self) {
        let session = self.session_id.clone();
        let target = self.target_mut();
        assert!(target.detach_session(&session).is_some());
        target.attach_session(session.clone());
        target.set_runtime_frontend_enabled(&session, true);
    }

    async fn emit(&mut self, outputs: TargetPreparedOutputs) -> Vec<serde_json::Value> {
        worker_target_background_events_async(&mut self.conn, outputs)
            .await
            .into_iter()
            .map(BackgroundProtocolEvent::into_protocol_message)
            .collect()
    }

    async fn startup_console(&mut self) -> RendererSharedWorkerConsoleMessage {
        assert!(
            self.fixture
                .context
                .run_dedicated_worker_if_waiting_for_debugger(self.instance)
        );
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let moli_core::RendererOutputTransportMessage::Publication(publication) =
                    self.fixture.output.recv().await.unwrap()
                else {
                    continue;
                };
                for record in publication.into_records() {
                    if let moli_core::RendererOutputItem::Observation(
                        moli_core::RendererProtocolObservation::DedicatedWorker(
                            RendererDedicatedWorkerObservation::Console {
                                instance_id,
                                message,
                            },
                        ),
                    ) = record.into_parts().1
                        && instance_id == self.instance
                    {
                        assert!(message.message.contains("dedicated-readiness-start"));
                        return message;
                    }
                }
            }
        })
        .await
        .expect("the real Worker must publish its startup console after debugger release")
    }
}

impl Drop for LoadingObserver {
    fn drop(&mut self) {
        self.fixture.service.shutdown();
        self.server.abort();
    }
}

#[tokio::test]
async fn native_dedicated_runtime_observer_survives_loading_without_releasing_bootstrap() {
    let mut observer = LoadingObserver::start().await;
    let ready = observer.loaded().await;
    let events = worker_target_background_events_async(&mut observer.conn, ready).await;
    let messages = events
        .into_iter()
        .map(BackgroundProtocolEvent::into_protocol_message)
        .collect::<Vec<_>>();
    let realm = messages
        .iter()
        .find(|message| message["method"] == "Runtime.executionContextCreated")
        .unwrap_or_else(|| panic!("Loaded must connect the early Runtime observer: {messages:?}"));
    assert_eq!(realm["sessionId"], observer.session_id);
    assert!(
        realm["params"]["context"]["uniqueId"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
    assert!(
        messages.iter().all(|message| message.get("id").is_none()),
        "internal enable ack must not escape: {messages:?}"
    );
    let pending = observer.conn.start_worker_runtime_protocol_message_for_session(
        Some(&observer.session_id),
        r#"{"id":72,"method":"Runtime.evaluate","params":{"expression":"typeof globalThis.bootstrapRan","returnByValue":true}}"#.into(),
    ).unwrap();
    let completed = pending.wait().await.unwrap();
    let messages = observer
        .conn
        .complete_worker_runtime_protocol_message_for_session(completed)
        .unwrap();
    assert!(messages.iter().any(|message| matches!(message, RendererRuntimeInspectorMessage::Protocol(message)
        if message.value()["id"] == 72 && message.value()["result"]["result"]["value"] == "undefined")),
        "Runtime readiness must not release bootstrap: {messages:?}");
}

#[tokio::test]
async fn native_dedicated_runtime_held_inspector_cannot_follow_a_reused_attachment() {
    let mut observer = LoadingObserver::start().await;
    let _ready = observer.loaded().await;
    let pending = observer
        .conn
        .start_worker_runtime_protocol_message_for_session(
            Some(&observer.session_id),
            r#"{"id":0,"method":"Runtime.enable"}"#.into(),
        )
        .unwrap();
    let completed = pending.wait().await.unwrap();
    let mut messages = observer
        .conn
        .complete_worker_runtime_protocol_message_for_session(completed)
        .unwrap();
    messages.retain(|message| !matches!(message, RendererRuntimeInspectorMessage::Protocol(message) if message.value().get("id").is_some()));
    assert!(
        !messages.is_empty(),
        "capture real inspector output before detach"
    );
    let held = record_dedicated_worker_target_runtime_inspector_messages(
        &observer.conn,
        &observer.context_id,
        observer.instance,
        Some(observer.session_id.clone()),
        messages,
    );
    observer.reuse_session();
    assert!(
        worker_target_background_events_async(&mut observer.conn, held)
            .await
            .is_empty(),
        "an old physical Runtime publication cannot be routed through a new same-named attachment"
    );
}

#[tokio::test]
async fn native_dedicated_runtime_snapshot_recovers_loaded_observer_without_duplicate_realm() {
    let mut observer = LoadingObserver::start().await;
    drop(observer.loaded().await);
    let snapshot = observer.fixture.service.handle().subscribe().unwrap().0;
    let events = observer.conn.project_browser_snapshot(snapshot).await;
    assert_eq!(
        events
            .iter()
            .filter(|event| (*event).clone().into_protocol_message()["method"]
                == "Runtime.executionContextCreated")
            .count(),
        1
    );
    let snapshot = observer.fixture.service.handle().subscribe().unwrap().0;
    let events = observer.conn.project_browser_snapshot(snapshot).await;
    assert!(
        events
            .into_iter()
            .all(|event| event.into_protocol_message()["method"]
                != "Runtime.executionContextCreated")
    );
}

#[tokio::test]
async fn native_dedicated_runtime_ready_cannot_reenable_a_disabled_observer() {
    let mut observer = LoadingObserver::start().await;
    let ready = observer.loaded().await;
    let session = observer.session_id.clone();
    observer
        .target_mut()
        .set_runtime_frontend_enabled(&session, false);
    let messages = observer.emit(ready).await;
    assert!(
        messages
            .iter()
            .all(|message| message["method"] != "Runtime.executionContextCreated")
    );
    assert!(observer.target_mut().runtime_execution_context().is_none());
}

#[tokio::test]
async fn native_dedicated_runtime_ready_cannot_bind_a_reused_attachment() {
    let mut observer = LoadingObserver::start().await;
    let ready = observer.loaded().await;
    observer.reuse_session();
    let messages = observer.emit(ready).await;
    assert!(
        messages
            .iter()
            .all(|message| message["method"] != "Runtime.executionContextCreated")
    );
    assert!(observer.target_mut().runtime_execution_context().is_none());
}

#[tokio::test]
async fn native_dedicated_runtime_ready_rejects_closed_core_before_projection_retirement() {
    let mut observer = LoadingObserver::start().await;
    let ready = observer.loaded().await;
    assert!(observer.fixture.context.remove().unwrap());
    let messages = observer.emit(ready).await;
    assert!(
        messages
            .iter()
            .all(|message| message["method"] != "Runtime.executionContextCreated")
    );
    assert!(observer.target_mut().runtime_execution_context().is_none());
    let snapshot = observer.fixture.service.handle().subscribe().unwrap().0;
    observer.conn.project_browser_snapshot(snapshot).await;
    assert!(
        observer
            .conn
            .shared_worker_target_for_session(Some(&observer.session_id))
            .is_none()
    );
}

#[tokio::test]
async fn native_dedicated_runtime_loaded_before_enable_completion_still_binds_observer() {
    let mut observer = LoadingObserver::start().await;
    let session = observer.session_id.clone();
    observer
        .target_mut()
        .set_runtime_frontend_enabled(&session, false);
    let params = json!({});
    let command = crate::conn::Cmd::for_test(
        Some(71),
        "Runtime.enable",
        &params,
        Some(&session),
        r#"{"id":71,"method":"Runtime.enable"}"#,
    );
    let Some(RuntimeCommandTaskStep::Pending(pending)) =
        try_start_runtime_command_dispatch(&mut observer.conn, &command)
    else {
        panic!("capture the real unavailable dispatch before opening the script gate");
    };
    let completed = pending.wait().await;
    let ready = observer.loaded().await;
    assert!(
        observer
            .emit(ready)
            .await
            .iter()
            .all(|message| message["method"] != "Runtime.executionContextCreated")
    );
    let RuntimeCommandTaskStep::Complete(plan) =
        complete_pending_runtime_command_at_response_boundary(
            &mut observer.conn,
            completed,
            &crate::conn::CommandResponseFlushContext::default(),
        )
        .await
    else {
        panic!("logical enable and physical binding must complete together");
    };
    let mut messages = Vec::new();
    plan.emit_into(&mut messages, Some(71), Some(&session));
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
        "{messages:?}"
    );
}

#[tokio::test]
async fn native_dedicated_runtime_flushes_early_console_once_after_real_realm() {
    let mut observer = LoadingObserver::start().await;
    let ready = observer.loaded().await;
    let console = observer.startup_console().await;
    let outputs = record_dedicated_worker_target_console_message(
        &mut observer.conn,
        &observer.context_id,
        observer.instance,
        console,
    );
    assert!(outputs.is_empty(), "console must wait for the real realm");
    let messages = observer.emit(ready).await;
    let realm = messages
        .iter()
        .position(|message| message["method"] == "Runtime.executionContextCreated")
        .unwrap();
    let logs = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message["method"] == "Runtime.consoleAPICalled")
        .collect::<Vec<_>>();
    assert_eq!(logs.len(), 1, "{messages:?}");
    assert!(
        realm < logs[0].0,
        "real realm must precede its buffered console"
    );
    assert_eq!(
        logs[0].1["params"]["args"][0]["value"],
        "dedicated-readiness-start"
    );
    let ready = dedicated_worker_runtime_ready_outputs(
        &observer.conn,
        &observer.context_id,
        observer.instance,
    );
    assert!(
        observer
            .emit(ready)
            .await
            .iter()
            .all(|message| message["method"] != "Runtime.consoleAPICalled")
    );
}

#[tokio::test]
async fn native_dedicated_runtime_held_console_cannot_follow_a_reused_attachment() {
    let mut observer = LoadingObserver::start().await;
    let ready = observer.loaded().await;
    assert!(
        observer
            .emit(ready)
            .await
            .iter()
            .any(|message| message["method"] == "Runtime.executionContextCreated")
    );
    let console = observer.startup_console().await;
    let held = record_dedicated_worker_target_console_message(
        &mut observer.conn,
        &observer.context_id,
        observer.instance,
        console,
    );
    assert!(!held.is_empty());
    observer.reuse_session();
    assert!(
        observer.emit(held).await.is_empty(),
        "old console output cannot consume the replacement attachment's cursor"
    );
    let session = observer.session_id.clone();
    assert_eq!(
        observer
            .target_mut()
            .pending_runtime_console_messages(&session)
            .len(),
        1
    );
}

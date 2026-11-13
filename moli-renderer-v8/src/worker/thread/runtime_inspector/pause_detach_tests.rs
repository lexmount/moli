use super::*;
use std::{pin::pin, time::Duration};

#[test]
fn worker_detach_stops_a_session_paused_inside_synchronous_evaluate() {
    assert_detach_during_pause(false);
}

#[test]
fn worker_detach_acknowledges_stop_without_resuming_another_debugger() {
    assert_detach_during_pause(true);
}

fn assert_detach_during_pause(peer_debugger: bool) {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let (wake_tx, _wake_rx) = mpsc::unbounded_channel();
    let (parent_tx, mut parent_rx) = mpsc::unbounded_channel();
    let runner = WorkerInspectorTaskRunner::new(
        wake_tx,
        std::sync::Arc::new(parking_lot::Mutex::new(Some(isolate.thread_safe_handle()))),
    );
    let inspector =
        WorkerRuntimeInspector::new(&mut isolate, runner.clone(), parent_tx, false, true);
    let context = {
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        inspector.attach_context(
            context,
            v8::Global::new(scope, context),
            "https://worker-pause-detach.test/worker.js",
        );
        v8::Global::new(scope, context)
    };
    for (session, method) in [
        ("A", "Debugger.enable"),
        (
            "B",
            if peer_debugger {
                "Debugger.enable"
            } else {
                "Runtime.enable"
            },
        ),
    ] {
        inspector
            .dispatch_protocol_message_scoped(
                Some(session),
                &json!({"id":1,"method":method}).to_string(),
                None,
            )
            .unwrap();
    }
    // The controller observes actual Debugger.paused notifications from the
    // owner pause loop. It never manufactures a pause or guesses with sleeps.
    let controller = std::thread::spawn(move || {
        struct UnblockOnFailure(WorkerInspectorTaskRunner);
        impl Drop for UnblockOnFailure {
            fn drop(&mut self) {
                if std::thread::panicking() {
                    self.0.dispose(WorkerInspectorError::Closed);
                }
            }
        }
        let _unblock = UnblockOnFailure(runner.clone());
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(async move {
                let phase = Cell::new("waiting for A's pause");
                let result = tokio::time::timeout(Duration::from_secs(5), async {
                    let mut frames = HashMap::new();
                    wait_for_paused_session(&mut parent_rx, &mut frames, "A").await;
                    let lifetime = runner.session_lifetime(Some("A"));
                    let mut destroyed = lifetime.retirement();
                    phase.set("detaching A");
                    runner
                        .append_detach(Some("A".into()))
                        .await
                        .expect("detach acknowledgement")
                        .expect("detach stops the retiring session");
                    if peer_debugger {
                        // A's pause notification is flushed from its active
                        // dispatch; B's pending notification can be forwarded
                        // only after an owner task (here detach) makes progress.
                        phase.set("waiting for B's pause");
                        let frame = wait_for_paused_session(&mut parent_rx, &mut frames, "B").await;
                        assert!(matches!(
                            destroyed.try_recv(),
                            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
                        ));
                        for (id, method, params) in [
                            (
                                2,
                                "Debugger.setVariableValue",
                                json!({"callFrameId":frame,"scopeNumber":0,"variableName":"value","newValue":{"value":42}}),
                            ),
                            (3, "Debugger.resume", json!({})),
                        ] {
                            phase.set(method);
                            let (tx, rx) = tokio::sync::oneshot::channel();
                            assert!(runner.append_protocol_message(
                                Some("B".into()),
                                json!({"id":id,"method":method,"params":params}).to_string(),
                                None,
                                tx,
                            ));
                            let messages = rx.await.unwrap().unwrap();
                            let response = messages
                                .into_iter()
                                .map(RendererRuntimeInspectorMessage::into_v8_inspector_message)
                                .find(|message| message["id"] == id)
                                .unwrap();
                            assert!(response.get("error").is_none(), "{response}");
                            assert_eq!(response["result"], json!({}));
                        }
                    }
                    phase.set("waiting for final native destruction");
                    destroyed.await.unwrap().unwrap();
                })
                .await;
                result.unwrap_or_else(|error| panic!("{}: {error}", phase.get()));
            });
    });
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Local::new(scope, &context);
    let scope = &mut v8::ContextScope::new(scope, context);
    let messages = inspector
        .dispatch_protocol_message(
            scope,
            Some("A"),
            r#"{"id":99,"method":"Runtime.evaluate","params":{"expression":"(() => { let value = 7; debugger; globalThis.afterDetach = value; return 1; })()"}}"#,
        )
        .unwrap();
    controller.join().expect("pause controller");
    assert!(!inspector.sessions.borrow().contains_key("A"));
    assert!(
        messages
            .iter()
            .all(|message| message.clone().into_v8_inspector_message()["id"] != 99)
    );
    let messages = inspector
        .dispatch_protocol_message(
            scope,
            Some("B"),
            r#"{"id":100,"method":"Runtime.evaluate","params":{"expression":"afterDetach","returnByValue":true}}"#,
        )
        .unwrap();
    assert!(messages.into_iter().any(|message| {
        let message = message.into_v8_inspector_message();
        message["id"] == 100
            && message["result"]["result"]["value"] == if peer_debugger { 42 } else { 7 }
    }));
}

async fn wait_for_paused_session(
    parent_rx: &mut mpsc::UnboundedReceiver<WorkerToParentMessage>,
    frames: &mut HashMap<String, String>,
    session: &str,
) -> String {
    loop {
        if let Some(frame) = frames.get(session) {
            return frame.clone();
        }
        let WorkerToParentMessage::RuntimeInspectorMessages(batches) =
            parent_rx.recv().await.expect("Worker notifications")
        else {
            continue;
        };
        for batch in batches {
            let Some(session) = batch.inspector_session_id else {
                continue;
            };
            for message in batch.messages {
                let message = message.into_v8_inspector_message();
                if message["method"] == "Debugger.paused"
                    && let Some(frame) = message["params"]["callFrames"][0]["callFrameId"].as_str()
                {
                    frames.insert(session.clone(), frame.to_owned());
                }
            }
        }
    }
}

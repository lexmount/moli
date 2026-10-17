use super::*;
use crate::script_vm::inspector::{
    context_registry::DocumentInspectorContextGroupId, v8_backend::RendererInspectorIsolateBackend,
};
use serde_json::json;
use std::{cell::RefCell, pin::pin, rc::Rc};

#[derive(Clone, Default)]
struct Notifications(Rc<RefCell<Vec<Value>>>);

impl v8::inspector::ChannelImpl for Notifications {
    fn send_response(&self, _: i32, message: v8::UniquePtr<v8::inspector::StringBuffer>) {
        self.send_notification(message);
    }

    fn send_notification(&self, message: v8::UniquePtr<v8::inspector::StringBuffer>) {
        self.0.borrow_mut().push(
            moli_v8_util::decode_inspector_protocol_message(message.as_ref().unwrap().string())
                .unwrap(),
        );
    }

    fn flush_protocol_notifications(&self) {}
}

impl InspectorSessionOutput for Notifications {
    fn capture_internal_response(&self, call_id: i32, dispatch: impl FnOnce()) -> Option<Value> {
        let start = self.0.borrow().len();
        dispatch();
        let mut messages = self.0.borrow_mut();
        let position = (start..messages.len()).find(|&index| messages[index]["id"] == call_id)?;
        Some(messages.remove(position))
    }
}

fn connect(
    backend: &mut RendererInspectorIsolateBackend,
    group: DocumentInspectorContextGroupId,
    bridge: &RendererInspectorPauseBridge,
    agent: RendererDevToolsAgentToken,
    state: &[u8],
) -> (RendererInspectorSession, Notifications) {
    let notifications = Notifications::default();
    let session = backend.connect_session(
        group,
        v8::inspector::Channel::new(Box::new(notifications.clone())),
        state,
    );
    (
        RendererInspectorSession::new(session, Some((bridge.clone(), agent))),
        notifications,
    )
}

fn context(
    isolate: &mut v8::Isolate,
    backend: &RendererInspectorIsolateBackend,
    group: DocumentInspectorContextGroupId,
) -> v8::Global<v8::Context> {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    backend.context_created_with_unique_id(context, group, b"test", b"https://example.test", b"{}");
    v8::Global::new(scope, context)
}

fn evaluate(isolate: &mut v8::Isolate, context: &v8::Global<v8::Context>, source: &str) {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let context = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, context);
    let source = v8::String::new(scope, source).unwrap();
    let script = v8::Script::compile(scope, source, None).unwrap();
    assert_eq!(
        crate::script_execution::execute_compiled_script(scope, script)
            .unwrap()
            .int32_value(scope),
        Some(42)
    );
}

fn dispatch(session: &RendererInspectorSession, request: Value) {
    session.dispatch_protocol_message(v8::inspector::StringView::from(
        request.to_string().as_bytes(),
    ));
}

fn state_json(session: &RendererInspectorSession) -> Value {
    serde_json::from_slice(&v8::crdtp::cbor_to_json(&session.state()).unwrap()).unwrap()
}

#[test]
fn replacement_prevents_v8_pause_notifications_and_cancellation_restores_them() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
    let bridge = RendererInspectorPauseBridge::default();
    let agent = RendererDevToolsAgentToken::allocate();
    let group = DocumentInspectorContextGroupId::next();
    let context = context(&mut isolate, &backend, group);
    let (first, first_notifications) = connect(&mut backend, group, &bridge, agent, b"{}");
    let (second, second_notifications) = connect(&mut backend, group, &bridge, agent, b"{}");
    for session in [&first, &second] {
        dispatch(session, json!({"id": 1, "method": "Debugger.enable"}));
        dispatch(session, json!({"id": 2, "method": "Runtime.enable"}));
    }
    for notifications in [&first_notifications, &second_notifications] {
        notifications.0.borrow_mut().clear();
    }
    assert!(bridge.begin_document_replacement(agent));
    first.sync_document_replacement();
    second.sync_document_replacement();
    evaluate(
        &mut isolate,
        &context,
        "console.log('preserved'); for (let i = 0; i < 4; ++i) { debugger; } 42",
    );
    for notifications in [&first_notifications, &second_notifications] {
        let messages = notifications.0.borrow();
        assert!(
            messages
                .iter()
                .any(|message| message["method"] == "Runtime.consoleAPICalled")
        );
        assert!(
            messages.iter().all(|message| {
                message["method"] != "Debugger.paused" && message["method"] != "Debugger.resumed"
            }),
            "V8's channel must never receive skipped pause/resume events: {messages:?}"
        );
    }

    bridge.finish_document_replacement(agent);
    first.sync_document_replacement();
    second.sync_document_replacement();
    evaluate(&mut isolate, &context, "debugger; 42");
    for notifications in [&first_notifications, &second_notifications] {
        let messages = notifications.0.borrow();
        for method in ["Debugger.paused", "Debugger.resumed"] {
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| message["method"] == method)
                    .count(),
                1,
                "canceling replacement must restore both frontend sessions: {messages:?}"
            );
        }
    }
}

#[test]
fn replacement_preserves_frontend_pause_policy_and_fresh_v8_state() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
    for previous in [false, true] {
        let bridge = RendererInspectorPauseBridge::default();
        let agent = RendererDevToolsAgentToken::allocate();
        let (session, _) = connect(
            &mut backend,
            DocumentInspectorContextGroupId::next(),
            &bridge,
            agent,
            b"{}",
        );
        dispatch(&session, json!({"id": 1, "method": "Debugger.enable"}));
        dispatch(
            &session,
            json!({"id": 2, "method": "Debugger.setSkipAllPauses", "params": {"skip": previous}}),
        );
        assert!(bridge.begin_document_replacement(agent));
        session.sync_document_replacement();
        assert!(
            session.skip_all_pauses(),
            "replacement must apply the native V8 flag"
        );
        dispatch(
            &session,
            json!({"id": 3, "method": "Profiler.setSamplingInterval", "params": {"interval": 937}}),
        );
        dispatch(&session, json!({"id": 4, "method": "Profiler.enable"}));
        let snapshot = state_json(&session);
        assert_eq!(snapshot["Debugger"]["skipAllPauses"], previous);
        assert_eq!(snapshot["Profiler"]["samplingInterval"], 937);
        assert!(
            session.skip_all_pauses(),
            "exporting state must leave the old session suppressed"
        );
        bridge.finish_document_replacement(agent);
        session.sync_document_replacement();
        assert_eq!(session.skip_all_pauses(), previous);

        assert!(bridge.begin_document_replacement(agent));
        dispatch(
            &session,
            json!({"id": 5, "method": "Debugger.setSkipAllPauses", "params": {"skip": !previous}}),
        );
        assert!(session.skip_all_pauses());
        assert_eq!(state_json(&session)["Debugger"]["skipAllPauses"], !previous);
        dispatch(
            &session,
            json!({"id": 6, "method": "Debugger.setSkipAllPauses", "params": {"skip": "invalid"}}),
        );
        assert_eq!(
            state_json(&session)["Debugger"]["skipAllPauses"],
            !previous,
            "a rejected frontend setting must preserve the previous policy"
        );
        bridge.finish_document_replacement(agent);
        session.sync_document_replacement();
        assert_eq!(session.skip_all_pauses(), !previous);
    }
}

#[test]
fn replacement_policy_survives_frontend_debugger_disable_and_reenable() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
    for previous in [false, true] {
        let bridge = RendererInspectorPauseBridge::default();
        let agent = RendererDevToolsAgentToken::allocate();
        let group = DocumentInspectorContextGroupId::next();
        let context = context(&mut isolate, &backend, group);
        let (session, notifications) = connect(&mut backend, group, &bridge, agent, b"{}");
        let frontend = |request: Value| {
            session
                .dispatch_with_runtime_defaults(&request.to_string(), &notifications)
                .unwrap();
        };
        frontend(json!({"id": 1, "method": "Debugger.enable"}));
        frontend(json!({
            "id": 2, "method": "Debugger.setSkipAllPauses", "params": {"skip": previous}
        }));
        assert!(bridge.begin_document_replacement(agent));
        frontend(json!({"id": -1, "method": "Runtime.enable"}));
        assert_eq!(state_json(&session)["Debugger"]["skipAllPauses"], previous);

        frontend(json!({"id": 3, "method": "Debugger.disable"}));
        frontend(json!({"id": 4, "method": "Debugger.enable"}));
        assert_eq!(state_json(&session)["Debugger"]["skipAllPauses"], false);
        assert!(session.skip_all_pauses());
        notifications.0.borrow_mut().clear();
        evaluate(&mut isolate, &context, "debugger; 42");
        assert!(
            notifications.0.borrow().iter().all(|message| {
                message["method"] != "Debugger.paused" && message["method"] != "Debugger.resumed"
            }),
            "reenabling Debugger must keep the outgoing document suppressed"
        );

        bridge.finish_document_replacement(agent);
        session.sync_document_replacement();
        assert!(!session.skip_all_pauses());
        evaluate(&mut isolate, &context, "debugger; 42");
        for method in ["Debugger.paused", "Debugger.resumed"] {
            assert_eq!(
                notifications
                    .0
                    .borrow()
                    .iter()
                    .filter(|message| message["method"] == method)
                    .count(),
                1,
                "cancellation must keep the frontend's policy after Debugger.disable"
            );
        }
    }
}

#[test]
fn old_replacement_scope_does_not_suppress_restored_document_breakpoints() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
    let bridge = RendererInspectorPauseBridge::default();
    let old_agent = RendererDevToolsAgentToken::allocate();
    let (old, _) = connect(
        &mut backend,
        DocumentInspectorContextGroupId::next(),
        &bridge,
        old_agent,
        b"{}",
    );
    dispatch(&old, json!({"id": 1, "method": "Debugger.enable"}));
    assert!(bridge.begin_document_replacement(old_agent));
    assert!(bridge.begin_document_replacement(old_agent));
    let snapshot = old.state();
    assert!(old.skip_all_pauses());
    bridge.finish_document_replacement(old_agent);
    old.sync_document_replacement();
    assert!(
        old.skip_all_pauses(),
        "another preparation still owns the old document's policy"
    );
    let group = DocumentInspectorContextGroupId::next();
    let context = context(&mut isolate, &backend, group);
    let (restored, notifications) = connect(
        &mut backend,
        group,
        &bridge,
        RendererDevToolsAgentToken::allocate(),
        &snapshot,
    );
    notifications.0.borrow_mut().clear();
    evaluate(&mut isolate, &context, "debugger; 42");
    let messages = notifications.0.borrow();
    assert!(
        messages
            .iter()
            .any(|message| message["method"] == "Debugger.paused"),
        "the restored document must pause while the old preparation scope is still alive: {messages:?}"
    );
    assert!(!restored.skip_all_pauses());
    bridge.finish_document_replacement(old_agent);
}

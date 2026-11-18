use super::*;
use crate::script_vm::inspector::agent_sessions::RendererInspectorSessions;

fn connect(
    backend: &mut RendererInspectorIsolateBackend,
    context_group_id: DocumentInspectorContextGroupId,
    outbound: InspectorOutbound,
    state: Option<&V8InspectorSessionState>,
) -> RendererInspectorSession {
    RendererInspectorSession::connect_native(
        backend,
        context_group_id,
        outbound,
        state.map(V8InspectorSessionState::as_bytes),
    )
}
use crate::inspector_session::dispatch_with_runtime_defaults;
use crate::script_vm::inspector::context_registry::DocumentInspectorContextRegistrationId;
use serde_json::Value;
use std::pin::pin;

fn state_json(state: &V8InspectorSessionState) -> Value {
    let json = v8::crdtp::cbor_to_json(state.as_bytes())
        .expect("V8 Inspector session state should be valid CBOR");
    serde_json::from_slice(&json).expect("V8 Inspector session state should contain JSON")
}

fn dispatch(session: &RendererInspectorSession, request: &str) -> Vec<Value> {
    let snapshot = session.outbound.len();
    let _response_capture = session.outbound.capture_dispatch_responses();
    dispatch_with_runtime_defaults(&session.session, request, &session.outbound)
        .expect("Inspector session dispatch");
    session
        .outbound
        .take_messages_after(snapshot)
        .into_iter()
        .map(|message| message.into_value())
        .collect()
}

fn assert_successful_response(messages: &[Value], call_id: u64) {
    let response = messages
        .iter()
        .find(|message| message.get("id").and_then(Value::as_u64) == Some(call_id))
        .unwrap_or_else(|| panic!("missing Inspector response {call_id}: {messages:?}"));
    assert!(
        response.get("error").is_none(),
        "Inspector response {call_id} failed: {response:?}"
    );
}

#[test]
fn debugger_pause_defaults_preserve_explicit_and_restored_settings() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
    let context_group_id = DocumentInspectorContextGroupId::next();
    let registration_id = DocumentInspectorContextRegistrationId::next();
    let default_context = {
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        backend.context_created_with_unique_id(
            context,
            context_group_id,
            b"https://example.test/",
            b"https://example.test",
            br#"{"isDefault":true,"type":"default"}"#,
        );
        v8::Global::new(scope, context)
    };
    backend.context_registry.set_default_context(
        context_group_id,
        default_context,
        registration_id,
    );
    for debugging in [false, true] {
        let outbound = || {
            InspectorOutbound::for_agent(RendererDevToolsAgentToken::allocate())
                .with_debugger_pauses_enabled(debugging)
        };
        let session = connect(&mut backend, context_group_id, outbound(), None);
        assert_eq!(
            state_json(&session.v8_state())["Debugger"]["skipAllPauses"],
            !debugging,
            "the native default must exist before the first frontend command",
        );
        assert!(session.outbound.values_after(0).is_empty());
        if !debugging {
            crate::inspector_session::tests::assert_debugger_defaults(|request| {
                dispatch(&session, request)
            });
        }
        assert_successful_response(
            &dispatch(&session, r#"{"id":6,"method":"Debugger.enable"}"#),
            6,
        );
        assert_eq!(
            state_json(&session.v8_state())["Debugger"]["skipAllPauses"]
                .as_bool()
                .unwrap_or(false),
            !debugging,
        );
        let initial_state = session.v8_state();
        drop(session);
        let session = connect(
            &mut backend,
            context_group_id,
            outbound(),
            Some(&initial_state),
        );
        assert_eq!(
            state_json(&session.v8_state())["Debugger"]["skipAllPauses"],
            !debugging,
            "restoring a session must preserve its existing pause default",
        );
        assert_successful_response(
            &dispatch(
                &session,
                r#"{"id":7,"method":"Debugger.setSkipAllPauses","params":{"skip":false}}"#,
            ),
            7,
        );
        assert_successful_response(
            &dispatch(&session, r#"{"id":8,"method":"Debugger.enable"}"#),
            8,
        );
        let state = session.v8_state();
        assert_eq!(state_json(&state)["Debugger"]["skipAllPauses"], false);
        drop(session);
        let restored = connect(&mut backend, context_group_id, outbound(), Some(&state));
        assert_successful_response(
            &dispatch(&restored, r#"{"id":9,"method":"Debugger.enable"}"#),
            9,
        );
        assert_eq!(
            state_json(&restored.v8_state())["Debugger"]["skipAllPauses"],
            false
        );
        assert_successful_response(
            &dispatch(&restored, r#"{"id":10,"method":"Debugger.disable"}"#),
            10,
        );
        assert_successful_response(
            &dispatch(&restored, r#"{"id":11,"method":"Debugger.enable"}"#),
            11,
        );
        assert_eq!(
            state_json(&restored.v8_state())["Debugger"]["skipAllPauses"],
            false,
            "disable and subsequent enable must follow V8's native reset behavior",
        );
    }
    let session = connect(
        &mut backend,
        context_group_id,
        InspectorOutbound::for_agent(RendererDevToolsAgentToken::allocate()),
        None,
    );
    assert_successful_response(
        &dispatch(
            &session,
            r#"{"id":12,"method":"Debugger.setSkipAllPauses","params":{"skip":false}}"#,
        ),
        12,
    );
    assert_successful_response(
        &dispatch(&session, r#"{"id":13,"method":"Debugger.enable"}"#),
        13,
    );
    assert_eq!(
        state_json(&session.v8_state())["Debugger"]["skipAllPauses"],
        false,
        "a client override before first enable must not be reset",
    );
}

#[test]
fn frontend_reattach_preserves_replay_state_without_exposing_internal_session() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
    let context_group_id = DocumentInspectorContextGroupId::next();
    let agent_token = RendererDevToolsAgentToken::allocate();
    let connection = RendererInspectorSessionConnection::new(
        backend.handle(),
        context_group_id,
        agent_token,
        None,
    );
    let session_key = DevToolsSessionKey::Attached("SID-agent-sessions".to_owned());
    let binding = RuntimeBindingRegistration {
        devtools_session: None,
        name: "bridge".to_owned(),
        execution_context_name: Some("utility".to_owned()),
    };
    let mut sessions = RendererInspectorSessions::default();
    sessions
        .frontend
        .set_runtime_bindings(session_key.clone(), std::slice::from_ref(&binding));

    let first_replay = sessions
        .frontend
        .prepare_frontend(&mut backend, &connection, session_key.clone())
        .run(|_, _, replay| replay);
    let second_replay = sessions
        .frontend
        .prepare_frontend(&mut backend, &connection, session_key.clone())
        .run(|_, _, replay| replay);
    assert_eq!(first_replay, vec![binding.clone()]);
    assert!(second_replay.is_empty(), "a live session must replay once");

    sessions
        .prepare_internal_runtime_evaluate(&mut backend, context_group_id, agent_token, None)
        .run(|_, _, _| {});
    assert_eq!(
        sessions.frontend.frontend_routes().len(),
        1,
        "the internal Runtime.evaluate session must not become frontend-visible"
    );
    assert_eq!(
        sessions.frontend.v8_states().len(),
        1,
        "the internal Runtime.evaluate session must not enter restore snapshots"
    );

    sessions.frontend.reattach_frontend(
        &mut backend,
        &connection,
        session_key.clone(),
        &V8InspectorSessionState::from_bytes(Vec::new()),
    );
    let replay_after_reattach = sessions
        .frontend
        .prepare_frontend(&mut backend, &connection, session_key)
        .run(|_, _, replay| replay);
    assert!(
        replay_after_reattach.is_empty(),
        "reattach must preserve session-local binding replay bookkeeping"
    );
}

#[test]
fn runtime_binding_configuration_is_agent_local() {
    let session_key = DevToolsSessionKey::Attached("SID-agent-local-config".to_owned());
    let binding = RuntimeBindingRegistration {
        devtools_session: None,
        name: "bridge".to_owned(),
        execution_context_name: Some("utility".to_owned()),
    };
    let mut first_agent_sessions = RendererInspectorSessions::default();
    let mut replacement_agent_sessions = RendererInspectorSessions::default();

    first_agent_sessions
        .frontend
        .set_runtime_bindings(session_key.clone(), std::slice::from_ref(&binding));

    assert_eq!(
        first_agent_sessions.frontend.runtime_bindings(&session_key),
        vec![binding.clone()]
    );
    assert!(
        replacement_agent_sessions
            .frontend
            .runtime_bindings(&session_key)
            .is_empty(),
        "a replacement agent must not inherit renderer-local configuration"
    );

    replacement_agent_sessions
        .frontend
        .set_runtime_bindings(session_key.clone(), std::slice::from_ref(&binding));
    assert_eq!(
        replacement_agent_sessions
            .frontend
            .runtime_bindings(&session_key),
        vec![binding],
        "the protocol restore path must configure the replacement agent explicitly"
    );
}

#[test]
fn inspector_session_state_round_trips_runtime_and_profiler_domains() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
    let context_group_id = DocumentInspectorContextGroupId::next();
    let registration_id = DocumentInspectorContextRegistrationId::next();

    let default_context = {
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        backend.context_created_with_unique_id(
            context,
            context_group_id,
            b"https://example.test/",
            b"https://example.test",
            br#"{"isDefault":true,"type":"default"}"#,
        );
        v8::Global::new(scope, context)
    };
    backend.context_registry.set_default_context(
        context_group_id,
        default_context,
        registration_id,
    );

    let first_attach = connect(
        &mut backend,
        context_group_id,
        InspectorOutbound::for_agent(RendererDevToolsAgentToken::allocate()),
        None,
    );
    assert_successful_response(
        &dispatch(&first_attach, r#"{"id":1,"method":"Runtime.enable"}"#),
        1,
    );
    assert_successful_response(
        &dispatch(
            &first_attach,
            r#"{"id":2,"method":"Profiler.setSamplingInterval","params":{"interval":937}}"#,
        ),
        2,
    );
    assert_successful_response(
        &dispatch(&first_attach, r#"{"id":3,"method":"Profiler.enable"}"#),
        3,
    );
    assert_successful_response(
        &dispatch(
            &first_attach,
            r#"{"id":4,"method":"Runtime.setCustomObjectFormatterEnabled","params":{"enabled":true}}"#,
        ),
        4,
    );

    assert_successful_response(
        &dispatch(
            &first_attach,
            r#"{"id":6,"method":"Runtime.setMaxCallStackSizeToCapture","params":{"size":50}}"#,
        ),
        6,
    );

    let state = first_attach.v8_state();
    assert!(
        !state.is_empty(),
        "stateful Inspector session should emit state"
    );
    let initial_state_json = state_json(&state);
    let initial_state_text = initial_state_json.to_string();
    assert!(
        initial_state_text.contains("runtime")
            && initial_state_text.contains("profiler")
            && initial_state_text.contains("937")
            && initial_state_text.contains("customObjectFormatterEnabled"),
        "state should contain Runtime and Profiler options: {initial_state_json}"
    );
    drop(first_attach);

    let restored = connect(
        &mut backend,
        context_group_id,
        InspectorOutbound::for_agent(RendererDevToolsAgentToken::allocate()),
        Some(&state),
    );
    let restored_messages = restored.outbound.values_after(0);
    let restored_context = restored_messages
        .iter()
        .find(|message| message.method() == Some("Runtime.executionContextCreated"))
        .unwrap_or_else(|| {
            panic!(
                "restored Runtime.enable should publish the existing context: {restored_messages:?}"
            )
        });
    assert_eq!(
        restored_context.value()["params"]["context"]["origin"],
        "https://example.test",
        "V8ContextInfo must carry the embedder-bound origin without outbound JSON rewriting"
    );
    let formatter_probe = serde_json::json!({
        "id": 5,
        "method": "Runtime.evaluate",
        "params": {
            "generatePreview": true,
            "expression": r#"(() => {
              globalThis.devtoolsFormatters = [{
                header(value) {
                  return value && value.restored
                    ? ["span", {}, "restored-formatter"]
                    : null;
                },
                hasBody() { return false; },
                body() { return null; }
              }];
              return {restored: true};
            })()"#
        }
    })
    .to_string();
    let formatter_messages = dispatch(&restored, &formatter_probe);
    let formatter_response = formatter_messages
        .iter()
        .find(|message| message.get("id").and_then(Value::as_u64) == Some(5))
        .unwrap_or_else(|| panic!("missing formatter probe response: {formatter_messages:?}"));
    assert!(
        formatter_response["result"]["result"]["customPreview"]["header"]
            .as_str()
            .is_some_and(|header| header.contains("restored-formatter")),
        "V8 should apply the restored custom formatter setting: {formatter_response:?}"
    );
    let restored_state = restored.v8_state();
    assert_successful_response(
        &dispatch(&restored, r#"{"id":7,"method":"Runtime.enable"}"#),
        7,
    );
    assert_eq!(
        state_json(&restored.v8_state()),
        state_json(&restored_state),
        "repeated Runtime.enable must preserve the restored stack-capture override"
    );
    assert_eq!(
        state_json(&restored_state),
        initial_state_json,
        "V8 should preserve opaque Runtime and Profiler state across reconnect"
    );
}

#[test]
fn runtime_reattach_before_replacement_context_reports_clear_then_created() {
    crate::ensure_v8_for_test();
    let runtime_state = {
        let mut isolate = v8::Isolate::new(Default::default());
        let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
        let context_group_id = DocumentInspectorContextGroupId::next();
        let registration_id = DocumentInspectorContextRegistrationId::next();
        let default_context = {
            let scope = pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            backend.context_created_with_unique_id(
                context,
                context_group_id,
                b"https://example.test/old",
                b"https://example.test",
                br#"{"isDefault":true,"type":"default"}"#,
            );
            v8::Global::new(scope, context)
        };
        backend.context_registry.set_default_context(
            context_group_id,
            default_context,
            registration_id,
        );

        let session = connect(
            &mut backend,
            context_group_id,
            InspectorOutbound::for_agent(RendererDevToolsAgentToken::allocate()),
            None,
        );
        assert_successful_response(
            &dispatch(&session, r#"{"id":1,"method":"Runtime.enable"}"#),
            1,
        );
        session.v8_state()
    };

    let mut replacement_isolate = v8::Isolate::new(Default::default());
    let mut replacement_backend = RendererInspectorIsolateBackend::new(&mut replacement_isolate);
    let replacement_context_group_id = DocumentInspectorContextGroupId::next();
    let replacement_registration_id = DocumentInspectorContextRegistrationId::next();
    let restored = connect(
        &mut replacement_backend,
        replacement_context_group_id,
        InspectorOutbound::for_agent(RendererDevToolsAgentToken::allocate()),
        Some(&runtime_state),
    );

    let replacement_context = {
        let scope = pin!(v8::HandleScope::new(&mut replacement_isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        replacement_backend.context_created_with_unique_id(
            context,
            replacement_context_group_id,
            b"https://example.test/new",
            b"https://example.test",
            br#"{"isDefault":true,"type":"default"}"#,
        );
        v8::Global::new(scope, context)
    };
    replacement_backend.context_registry.set_default_context(
        replacement_context_group_id,
        replacement_context,
        replacement_registration_id,
    );

    let messages = restored.outbound.values_after(0);
    let cleared_index = messages
        .iter()
        .position(|message| message.method() == Some("Runtime.executionContextsCleared"))
        .unwrap_or_else(|| {
            panic!("restored Runtime session should clear old contexts: {messages:?}")
        });
    let created_index = messages
        .iter()
        .position(|message| message.method() == Some("Runtime.executionContextCreated"))
        .unwrap_or_else(|| {
            panic!("restored Runtime session should report the new context: {messages:?}")
        });
    assert!(
        cleared_index < created_index,
        "replacement context must be reported after old contexts are cleared: {messages:?}"
    );
}

#[test]
fn inspector_session_accepts_empty_reattach_state() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
    let context_group_id = DocumentInspectorContextGroupId::next();
    let empty_state = V8InspectorSessionState::from_bytes(Vec::new());

    let session = connect(
        &mut backend,
        context_group_id,
        InspectorOutbound::for_agent(RendererDevToolsAgentToken::allocate()),
        Some(&empty_state),
    );
    assert_successful_response(
        &dispatch(&session, r#"{"id":1,"method":"Runtime.enable"}"#),
        1,
    );
    assert!(
        v8::crdtp::cbor_to_json(&session.session.state()).is_some(),
        "empty reattach should produce a valid V8 state cookie after dispatch"
    );
}

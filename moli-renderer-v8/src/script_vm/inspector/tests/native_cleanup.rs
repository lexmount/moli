use super::*;
use std::pin::pin;

fn eval_native(
    isolate: &mut v8::Isolate,
    context: &v8::Global<v8::Context>,
    source: &str,
) -> String {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let context = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, context);
    let source = v8::String::new(scope, source).unwrap();
    let script = v8::Script::compile(scope, source, None).unwrap();
    crate::script_execution::execute_compiled_script(scope, script)
        .unwrap()
        .to_rust_string_lossy(scope)
}

#[test]
fn cancelled_helper_releases_late_native_results_before_deferred_publication() {
    // Both cancellation paths must work even when a recorder would normally
    // park the native response before the Browser sees receiver cancellation.
    for remove_callback in [false, true] {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
        let page = DocumentInspectorBinding::new(backend.handle());
        let (context, context_id) = {
            let scope = pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            backend.context_created_with_unique_id(
                context,
                page.agent.context_group_id(),
                b"helper",
                b"https://helper.test",
                b"{}",
            );
            (
                v8::Global::new(scope, context),
                v8::inspector::V8Inspector::execution_context_id(context),
            )
        };
        let (tx, receiver) = tokio::sync::oneshot::channel();
        let outbound = page.with_session_and_outbound(
            &mut backend, PageInspectorSessionTarget::Frontend(Some("helper")),
            |session, outbound, _| {
                outbound.register_response_callback(RendererRuntimeInspectorResponseSender::new(71, tx)
                    .with_abandoned_object_group_cleanup(Some("private-operation".into())));
                let request = json!({"id":71,"method":"Runtime.evaluate","params":{
                    "contextId":context_id,"awaitPromise":true,"objectGroup":"private-operation",
                    "expression":"new Promise(resolve => { globalThis.finishHelper = () => { delete globalThis.finishHelper; const result = {answer:42}; globalThis.helperResult = new WeakRef(result); resolve(result); }; })"
                }}).to_string();
                session.dispatch_protocol_message(v8::inspector::StringView::from(request.as_bytes()));
                // Emulate lease cleanup winning before the async native result
                // exists. Native terminal cleanup must release the later root.
                session.release_object_group(v8::inspector::StringView::from(b"private-operation".as_slice()));
                outbound
            },
        );
        assert_eq!(outbound.response_callback_counts(), (1, 0));
        drop(receiver);
        if remove_callback {
            outbound.cancel_response_callback(71);
        }
        let recorder = RendererRuntimeCommandOutputRecorder::new(None, 71);
        outbound.begin_runtime_command_output(recorder.clone());
        eval_native(&mut isolate, &context, "finishHelper(); 'completed'");
        isolate.perform_microtask_checkpoint();
        outbound.end_runtime_command_output();
        assert_eq!(
            outbound.response_callback_counts(),
            (0, 0),
            "native terminal consumed the abandoned operation"
        );
        assert!(
            recorder.finish().messages().is_empty(),
            "cancelled responses must not be parked for later publication"
        );
        isolate.clear_kept_objects();
        isolate.low_memory_notification();
        assert_eq!(
            eval_native(
                &mut isolate,
                &context,
                "String(helperResult.deref() === undefined)"
            ),
            "true",
            "the late native result must not retain its Inspector root"
        );
    }
}

#[test]
fn late_old_native_channel_cannot_release_a_reconnected_sessions_group() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
    let page = DocumentInspectorBinding::new(backend.handle());
    let context = {
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        backend.context_created_with_unique_id(
            context,
            page.agent.context_group_id(),
            b"reattach",
            b"https://helper.test",
            b"{}",
        );
        v8::Global::new(scope, context)
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    let old_outbound = page.with_session_and_outbound(
        &mut backend,
        PageInspectorSessionTarget::Frontend(Some("reused")),
        |_, outbound, _| {
            outbound.register_response_callback(
                RendererRuntimeInspectorResponseSender::new(72, tx)
                    .with_abandoned_object_group_cleanup(Some("same-group".into())),
            );
            outbound.cancel_response_callback(72);
            outbound
        },
    );
    drop(rx);
    let state = page.v8_session_state(Some("reused")).unwrap();
    page.agent.reattach_frontend(
        &mut backend,
        DevToolsSessionKey::Attached("reused".into()),
        &state,
    );
    let context_id = {
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        v8::inspector::V8Inspector::execution_context_id(v8::Local::new(scope, &context))
    };
    let object_id = page.with_session_and_outbound(&mut backend, PageInspectorSessionTarget::Frontend(Some("reused")), |session, outbound, _| {
        let _capture = outbound.capture_dispatch_responses();
        let request = json!({"id":73,"method":"Runtime.evaluate","params":{"expression":"({owner:'new'})","contextId":context_id,"objectGroup":"same-group"}}).to_string();
        session.dispatch_protocol_message(v8::inspector::StringView::from(request.as_bytes()));
        let messages = outbound.take_pending_messages();
        messages.iter().find(|m| m.id_i64()==Some(73)).unwrap().value()["result"]["result"]["objectId"].as_str().unwrap().to_owned()
    });
    old_outbound.push_response_value(72, json!({"id":72,"result":{}}));
    page.with_session_and_outbound(&mut backend, PageInspectorSessionTarget::Frontend(Some("reused")), |session, _, _| {
        let scope = pin!(v8::HandleScope::new(&mut isolate)); let scope = &mut scope.init();
        let context = v8::Local::new(scope, &context);
        let scope = &mut v8::ContextScope::new(scope, context);
        assert!(session.unwrap_object(scope, v8::inspector::StringView::from(object_id.as_bytes())).is_ok(), "old cleanup cannot follow the shared outbound store to the replacement native session");
    });
}

use moli_protocol::devtools_runtime::{
    AutomationEvent, BrowserDownloadProgressEvent, BrowserDownloadWillBeginEvent,
    DevToolsBrowserContextId, DevToolsFrameId, DevToolsLoaderId, DevToolsNavigationId,
    DevToolsNetworkInterceptId, DevToolsNetworkResourceType, DevToolsRealmId, DevToolsRemoteValue,
    DevToolsRequestId, DevToolsStackCallFrame, DevToolsStackTrace, DevToolsTargetId,
    DevToolsTargetInfo, DevToolsTargetKind, LogEntryEvent, NavigationFrameEvent,
    NavigationFrameEventKind, NavigationLifecycleEvent, NetworkAuthChallengeEvent,
    NetworkRequestEvent, PageFileChooserOpenedEvent, PageJavaScriptDialogOpeningEvent,
    RuntimeConsoleEvent, RuntimeExecutionContextEvent, RuntimeExecutionContextsClearedEvent,
    SameDocumentNavigationEvent, ScriptMessageEvent, TargetLifecycleEvent, UserPromptClosedEvent,
    webdriver_bidi_node_shared_id_for_backend_node_id,
};
use serde_json::{Value, json};

fn bidi_connection_with_session() -> (super::BidiConnectionState, super::BidiSessionRegistry) {
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
    let session = state.handle_message_with_session_registry(
        json!({
            "id": 1_u64,
            "method": "session.new",
            "params": {}
        }),
        &mut registry,
    );
    assert_eq!(session.response["type"], json!("success"));
    record_bidi_context_tree(
        &mut state,
        &[("FRAME-1", "default"), ("FRAME-2", "default")],
    );
    (state, registry)
}

fn record_bidi_context_tree(state: &mut super::BidiConnectionState, contexts: &[(&str, &str)]) {
    let contexts = contexts
        .iter()
        .map(|(context, user_context)| {
            json!({
                "context": context,
                "clientWindow": context,
                "userContext": user_context,
                "children": []
            })
        })
        .collect::<Vec<_>>();
    state.record_bidi_command_response(
        Some("browsingContext.getTree"),
        None,
        &json!({
            "type": "success",
            "result": {
                "contexts": contexts
            }
        }),
    );
}

fn service_worker_target_info() -> DevToolsTargetInfo {
    DevToolsTargetInfo {
        target_id: Some(DevToolsTargetId::from("TID-service-worker")),
        kind: DevToolsTargetKind::ServiceWorker,
        title: "Service Worker https://example.test/service-worker.js".to_owned(),
        url: "https://example.test/service-worker.js".to_owned(),
        attached: false,
        opener_id: None,
        opener_frame_id: None,
        can_access_opener: false,
        browser_context_id: Some(DevToolsBrowserContextId::from("BID-service-worker")),
        moli_popup_id: None,
    }
}

fn shared_worker_target_info() -> DevToolsTargetInfo {
    DevToolsTargetInfo {
        target_id: Some(DevToolsTargetId::from("TID-shared-worker")),
        kind: DevToolsTargetKind::SharedWorker,
        title: "shared-worker-smoke".to_owned(),
        url: "https://example.test/shared-worker.js".to_owned(),
        attached: false,
        opener_id: None,
        opener_frame_id: None,
        can_access_opener: false,
        browser_context_id: Some(DevToolsBrowserContextId::from("BID-shared-worker")),
        moli_popup_id: None,
    }
}

fn record_bidi_user_context(state: &mut super::BidiConnectionState, user_context: &str) {
    state.record_bidi_command_response(
        Some("browser.createUserContext"),
        None,
        &json!({
            "type": "success",
            "result": {
                "userContext": user_context
            }
        }),
    );
}

fn bidi_session_command_response(
    state: &mut super::BidiConnectionState,
    registry: &mut super::BidiSessionRegistry,
    id: u64,
    method: &str,
    params: Value,
) -> Value {
    state
        .handle_message_with_session_registry(
            json!({
                "id": id,
                "method": method,
                "params": params
            }),
            registry,
        )
        .response
}

fn bidi_session_channel_command_response(
    state: &mut super::BidiConnectionState,
    registry: &mut super::BidiSessionRegistry,
    id: u64,
    method: &str,
    params: Value,
    channel: &str,
) -> Value {
    state
        .handle_message_with_session_registry(
            json!({
                "id": id,
                "method": method,
                "params": params,
                "goog:channel": channel
            }),
            registry,
        )
        .response
}

fn assert_bidi_session_command_error(method: &str, params: Value, error: &str) {
    let (mut state, mut registry) = bidi_connection_with_session();
    let response = bidi_session_command_response(&mut state, &mut registry, 2, method, params);
    assert_eq!(response["type"], json!("error"), "{method} response");
    assert_eq!(response["error"], json!(error), "{method} response");
}

fn assert_bidi_adapter_invalid(method: &str, params: Value) -> String {
    let command = super::parse_bidi_command(json!({
        "id": 99,
        "method": method,
        "params": params,
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let error = super::devtools_command_from_bidi_command(&command, &context)
        .expect_err("command should fail validation");

    assert_eq!(error.code, super::BidiErrorCode::InvalidArgument);
    error.message
}

fn call_function_with_channel_value(channel_value: Value) -> Value {
    json!({
        "functionDeclaration": "(arg) => arg",
        "target": {"context": "TARGET-1"},
        "arguments": [{
            "type": "channel",
            "value": channel_value
        }]
    })
}

fn add_preload_script_with_channel_value(channel_value: Value) -> Value {
    json!({
        "functionDeclaration": "() => {}",
        "arguments": [{
            "type": "channel",
            "value": channel_value
        }]
    })
}

mod browsing_context_commands;
mod log_and_runtime_events;
mod network_event_subscriptions;
mod realm_and_context_events;
mod script_storage_and_validation;
mod session_and_network_commands;
mod subscription_lifecycle;

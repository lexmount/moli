use super::*;

pub(super) fn mouse_input_json_witness(reply: &RendererPageReply) -> serde_json::Value {
    let RendererPageReply::RuntimeEvaluationResult(result) = reply else {
        panic!(
            "expected JSON-string input witness; unexpected reply variant: {:?}",
            std::mem::discriminant(reply)
        );
    };
    let payload = result.as_protocol_payload();
    let serialized = payload
        .get("value")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("expected JSON-string input witness; raw payload: {payload:?}"));
    serde_json::from_str(serialized).unwrap_or_else(|error| {
        panic!("invalid input witness JSON: {error}; raw payload: {payload:?}")
    })
}

pub(super) async fn assert_window_performance_surface_for_test(
    page: &RendererPageHandle,
    phase: &str,
) {
    let (surface, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify({
  sameObject: performance === performance,
  prototype: Object.getPrototypeOf(performance) === Performance.prototype,
  finiteTimeOrigin: Number.isFinite(performance.timeOrigin)
})"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .unwrap_or_else(|error| panic!("{phase} Performance surface should evaluate: {error}"));
    assert_eq!(
        renderer_json_value(surface),
        Some(serde_json::json!(
            r#"{"sameObject":true,"prototype":true,"finiteTimeOrigin":true}"#
        )),
        "{phase} must bind Performance to its realm's canonical Window"
    );
}

pub(super) async fn default_execution_context_id_for_test(
    page: &RendererPageHandle,
) -> anyhow::Result<Option<i64>> {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::DefaultExecutionContextId)
        .await?;
    match reply {
        RendererPageReply::OptionalExecutionContextId(execution_context_id) => {
            Ok(execution_context_id)
        }
        _ => Err(anyhow::anyhow!(
            "expected DefaultExecutionContextId to return an optional execution context id"
        )),
    }
}

pub(super) async fn child_frame_id_for_default_context_id_for_test(
    page: &RendererPageHandle,
    execution_context_id: i64,
) -> anyhow::Result<String> {
    let (reply, _) = page
        .run_async_command(
            RendererPageCommand::ChildFrameIdForDefaultExecutionContextId(execution_context_id),
        )
        .await?;
    match reply {
        RendererPageReply::OptionalString(Some(frame_id)) => Ok(frame_id),
        _ => Err(anyhow::anyhow!(
            "expected child frame id lookup to return an optional string"
        )),
    }
}

pub(super) fn runtime_protocol_response_by_id(
    messages: &[serde_json::Value],
    response_id: i64,
) -> Option<&serde_json::Value> {
    messages
        .iter()
        .find(|message| message.get("id").and_then(serde_json::Value::as_i64) == Some(response_id))
}

pub(super) fn runtime_inspector_message_protocol_message_for_test(
    message: RendererRuntimeInspectorMessage,
) -> serde_json::Value {
    match message {
        RendererRuntimeInspectorMessage::Protocol(message) => message.into_value(),
        RendererRuntimeInspectorMessage::RuntimeContext(event) => {
            runtime_context_restore_event_protocol_message_for_test(event)
        }
    }
}

pub(super) fn runtime_context_restore_event_protocol_message_for_test(
    event: crate::protocol_types::RuntimeContextRestoreEvent,
) -> serde_json::Value {
    match event {
        crate::protocol_types::RuntimeContextRestoreEvent::Created(event) => {
            let crate::protocol_types::RuntimeExecutionContextRestoreEvent {
                context_id,
                realm_id,
                frame_id,
                origin,
                name,
                is_default,
                context_type,
                grant_universal_access,
            } = event;
            let mut aux_data = serde_json::Map::new();
            if let Some(frame_id) = frame_id {
                aux_data.insert("frameId".to_owned(), serde_json::json!(frame_id));
            }
            aux_data.insert("isDefault".to_owned(), serde_json::json!(is_default));
            aux_data.insert("type".to_owned(), serde_json::json!(context_type));
            if let Some(grant_universal_access) = grant_universal_access {
                aux_data.insert(
                    "grantUniversalAccess".to_owned(),
                    serde_json::json!(grant_universal_access),
                );
            }
            serde_json::json!({
                "method": "Runtime.executionContextCreated",
                "params": {
                    "context": {
                        "id": context_id,
                        "uniqueId": realm_id,
                        "origin": origin,
                        "name": name,
                        "auxData": serde_json::Value::Object(aux_data),
                    },
                },
            })
        }
        crate::protocol_types::RuntimeContextRestoreEvent::Destroyed(event) => {
            let crate::protocol_types::RuntimeExecutionContextRestoreEvent {
                context_id,
                realm_id,
                ..
            } = event;
            serde_json::json!({
                "method": "Runtime.executionContextDestroyed",
                "params": {
                    "executionContextId": context_id,
                    "executionContextUniqueId": realm_id,
                },
            })
        }
        crate::protocol_types::RuntimeContextRestoreEvent::Cleared(_event) => {
            serde_json::json!({
                "method": "Runtime.executionContextsCleared",
                "params": {},
            })
        }
    }
}

pub(super) fn runtime_execution_context_ids(messages: &[serde_json::Value]) -> Vec<i64> {
    messages
        .iter()
        .filter(|message| {
            message.get("method") == Some(&serde_json::json!("Runtime.executionContextCreated"))
        })
        .filter_map(|message| message["params"]["context"]["id"].as_i64())
        .collect()
}

pub(super) fn runtime_execution_context_unique_ids(messages: &[serde_json::Value]) -> Vec<&str> {
    messages
        .iter()
        .filter(|message| {
            message.get("method") == Some(&serde_json::json!("Runtime.executionContextCreated"))
        })
        .filter_map(|message| message["params"]["context"]["uniqueId"].as_str())
        .collect()
}

pub(super) fn runtime_execution_context_by_id(
    messages: &[serde_json::Value],
    context_id: i64,
) -> Option<&serde_json::Value> {
    messages
        .iter()
        .filter(|message| {
            message.get("method") == Some(&serde_json::json!("Runtime.executionContextCreated"))
        })
        .map(|message| &message["params"]["context"])
        .find(|context| context["id"].as_i64() == Some(context_id))
}

pub(super) fn runtime_default_context_count(messages: &[serde_json::Value]) -> usize {
    messages
        .iter()
        .filter(|message| {
            message.get("method") == Some(&serde_json::json!("Runtime.executionContextCreated"))
                && message["params"]["context"]["auxData"]["isDefault"] == serde_json::json!(true)
                && message["params"]["context"]["auxData"]["type"] == serde_json::json!("default")
                && message["params"]["context"]["auxData"]["frameId"]
                    .as_str()
                    .is_none()
        })
        .count()
}

pub(super) async fn set_stored_document_start_scripts_for_test(
    page: &RendererPageHandle,
    scripts: Vec<crate::DocumentStartScript>,
) -> anyhow::Result<()> {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::SetStoredDocumentStartScripts(scripts))
        .await?;
    match reply {
        RendererPageReply::Unit => Ok(()),
        _ => Err(anyhow::anyhow!(
            "expected SetStoredDocumentStartScripts to return unit reply"
        )),
    }
}

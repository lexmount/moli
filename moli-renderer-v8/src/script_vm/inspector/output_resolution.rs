use moli_page_types::MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH;
use serde_json::{Value, json};
use std::rc::{Rc, Weak};

use super::context_registry::{DocumentInspectorContextGroupId, DocumentInspectorContextRegistry};

use crate::{
    runtime::{RendererRuntimeInspectorMessage, RendererRuntimeInspectorMessageBatch},
    script_vm::ScriptVm,
};

/// Finalizes embedder-owned `RemoteObject` metadata on the V8 session's own
/// output path.
///
/// Chromium supplies this through `V8InspectorClient::valueSubtype`. The
/// prebuilt rusty_v8 bridge does not expose that callback, so Moli resolves
/// object ids against the same V8 session before its channel publishes the
/// serialized message. No Page-owner command or protocol-side round trip is
/// involved.
#[derive(Clone)]
pub(super) struct RendererInspectorMessageFinalizer {
    isolate: v8::UnsafeRawIsolatePtr,
    context_registry: DocumentInspectorContextRegistry,
    context_group_id: DocumentInspectorContextGroupId,
    session: Weak<v8::inspector::V8InspectorSession>,
}

impl RendererInspectorMessageFinalizer {
    pub(super) fn new(
        isolate: v8::UnsafeRawIsolatePtr,
        context_registry: DocumentInspectorContextRegistry,
        context_group_id: DocumentInspectorContextGroupId,
        session: &Rc<v8::inspector::V8InspectorSession>,
    ) -> Self {
        Self {
            isolate,
            context_registry,
            context_group_id,
            session: Rc::downgrade(session),
        }
    }

    pub(super) fn finalize(&self, message: &mut Value) {
        let mut paths = Vec::new();
        collect_remote_object_paths(message, "", &mut paths);
        for path in paths {
            let Some(object_id) = message.pointer(&path).and_then(|remote_object| {
                let object_id = remote_object.get("objectId")?.as_str()?;
                let object_like = remote_object
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| matches!(kind, "object" | "function"));
                (object_like && remote_object.get("subtype").is_none())
                    .then(|| object_id.to_owned())
            }) else {
                continue;
            };
            if !self.remote_object_is_node(&object_id) {
                continue;
            }
            if let Some(remote_object) = message.pointer_mut(&path).and_then(Value::as_object_mut) {
                remote_object.insert("subtype".to_owned(), json!("node"));
            }
        }
    }

    fn remote_object_is_node(&self, object_id: &str) -> bool {
        let Some(session) = self.session.upgrade() else {
            return false;
        };
        self.context_registry
            .with_default_context(self.context_group_id, |default_context| {
                let mut isolate_ptr = self.isolate;
                let isolate =
                    unsafe { v8::Isolate::ref_from_raw_isolate_ptr_mut(&mut isolate_ptr) };
                let scope = std::pin::pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, default_context);
                let scope = &mut v8::ContextScope::new(scope, context);
                let Ok(unwrapped) = session
                    .unwrap_object(scope, v8::inspector::StringView::from(object_id.as_bytes()))
                else {
                    return false;
                };
                let Ok(object) = v8::Local::<v8::Object>::try_from(unwrapped.value) else {
                    return false;
                };
                let scope = &mut v8::ContextScope::new(scope, unwrapped.context);
                crate::native_bridge::object_is_node_wrapper_or_detached(scope, object)
            })
            .unwrap_or(false)
    }
}

fn collect_remote_object_paths(value: &Value, path: &str, out: &mut Vec<String>) {
    let mut stack = vec![(value, path.to_owned(), MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH)];
    while let Some((value, path, remaining_depth)) = stack.pop() {
        let Some(next_depth) = remaining_depth.checked_sub(1) else {
            continue;
        };
        match value {
            Value::Object(map) => {
                if map.get("objectId").and_then(Value::as_str).is_some()
                    && map.get("type").and_then(Value::as_str).is_some()
                {
                    out.push(path.clone());
                }
                let children = map.iter().collect::<Vec<_>>();
                for (key, child) in children.into_iter().rev() {
                    let escaped_key = key.replace('~', "~0").replace('/', "~1");
                    stack.push((child, format!("{path}/{escaped_key}"), next_depth));
                }
            }
            Value::Array(values) => {
                for index in (0..values.len()).rev() {
                    stack.push((&values[index], format!("{path}/{index}"), next_depth));
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
}

impl ScriptVm {
    /// Resolves Page-backed Inspector metadata before a message batch enters
    /// the renderer output stream.
    ///
    /// Protocol projection can run while the Page owner is parked in a modal
    /// dialog or debugger loop. It must therefore consume a frozen message and
    /// never call back into that Page merely to translate context ids or detect
    /// DOM-node remote objects.
    pub(in crate::script_vm) fn resolve_runtime_inspector_batch_for_publication(
        &mut self,
        batch: &mut RendererRuntimeInspectorMessageBatch,
    ) {
        let raw_messages = batch
            .messages
            .iter()
            .cloned()
            .map(RendererRuntimeInspectorMessage::into_v8_inspector_message)
            .collect::<Vec<_>>();
        self.page_inspector
            .record_execution_context_state(&raw_messages, self.root_frame_id.as_deref());
        self.page_isolated_world_contexts
            .record_inspector_context_state(&raw_messages, self.root_frame_id.as_deref());

        for message in &mut batch.messages {
            let RendererRuntimeInspectorMessage::Protocol(message) = message else {
                continue;
            };
            let mut value = message.value_mut();
            self.resolve_runtime_event_context_id_for_publication(&mut value);
        }

        if batch.agent_token == self.page_inspector.agent_token() {
            batch.v8_state_update =
                self.inspector_v8_session_state(batch.session.wire_session_id());
        }
    }

    fn resolve_runtime_event_context_id_for_publication(&self, message: &mut Value) {
        let context_pointer = match message.get("method").and_then(Value::as_str) {
            Some("Runtime.consoleAPICalled") => "/params/executionContextId",
            Some("Runtime.exceptionThrown") => "/params/exceptionDetails/executionContextId",
            _ => return,
        };
        let Some(inspector_context_id) = message.pointer(context_pointer).and_then(Value::as_i64)
        else {
            return;
        };
        let Some(compatibility_context_id) = self
            .page_isolated_world_contexts
            .execution_context_id_for_inspector_context(inspector_context_id)
        else {
            return;
        };
        if let Some(context_id) = message.pointer_mut(context_pointer) {
            *context_id = json!(compatibility_context_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::collect_remote_object_paths;
    use moli_page_types::MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH;
    use serde_json::json;

    #[test]
    fn remote_object_scan_is_bounded_and_preserves_json_pointer_paths() {
        let value = json!({
            "a/b": [{"type": "object", "objectId": "first"}],
            "tail~": {"type": "object", "objectId": "second"},
        });
        let mut paths = Vec::new();
        collect_remote_object_paths(&value, "", &mut paths);
        assert_eq!(paths, ["/a~1b/0", "/tail~0"]);

        let mut too_deep = json!({"type": "object", "objectId": "too-deep"});
        for _ in 0..MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH {
            too_deep = json!({"child": too_deep});
        }
        paths.clear();
        collect_remote_object_paths(&too_deep, "", &mut paths);
        assert!(paths.is_empty());
    }
}

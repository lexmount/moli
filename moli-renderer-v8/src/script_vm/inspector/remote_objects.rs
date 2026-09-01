use moli_page_types::MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH;
use serde_json::Value;
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

/// Completes embedder-owned `Runtime.RemoteObject` fields at the renderer's
/// native V8 Inspector boundary.
///
/// V8 knows how to classify its built-in objects, but the vendored Inspector
/// client binding does not expose Blink's `valueSubtype()` hook for Moli's DOM
/// wrappers. Resolve those wrappers synchronously while the Channel callback's
/// native handle scope is still alive. Once a message leaves this boundary it
/// is immutable protocol output; downstream routing must never query the Page
/// that produced it.
#[derive(Clone, Default)]
pub(super) struct InspectorRemoteObjectCompleter {
    session: Rc<RefCell<Weak<v8::inspector::V8InspectorSession>>>,
}

impl InspectorRemoteObjectCompleter {
    pub(super) fn bind_session(&self, session: &Rc<v8::inspector::V8InspectorSession>) {
        *self.session.borrow_mut() = Rc::downgrade(session);
    }

    pub(super) fn clear_session(&self) {
        *self.session.borrow_mut() = Weak::new();
    }

    pub(super) fn complete_message(&self, message: &mut Value) {
        let Some(session) = self.session.borrow().upgrade() else {
            return;
        };
        for (path, object_id) in unclassified_remote_objects(message) {
            if !inspector_object_is_node(&session, &object_id) {
                continue;
            }
            if let Some(remote_object) = message.pointer_mut(&path).and_then(Value::as_object_mut) {
                remote_object.insert("subtype".to_owned(), Value::String("node".to_owned()));
            }
        }
    }
}

fn unclassified_remote_objects(message: &Value) -> Vec<(String, String)> {
    let mut objects = Vec::new();
    let mut stack = vec![(message, String::new(), MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH)];
    while let Some((value, path, remaining_depth)) = stack.pop() {
        let Some(next_depth) = remaining_depth.checked_sub(1) else {
            continue;
        };
        match value {
            Value::Object(object) => {
                let is_unclassified_remote_object = object.get("subtype").is_none()
                    && object.get("type").and_then(Value::as_str) == Some("object");
                if is_unclassified_remote_object
                    && let Some(object_id) = object.get("objectId").and_then(Value::as_str)
                {
                    objects.push((path.clone(), object_id.to_owned()));
                }
                for (key, child) in object.iter().rev() {
                    let key = key.replace('~', "~0").replace('/', "~1");
                    stack.push((child, format!("{path}/{key}"), next_depth));
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
    objects
}

fn inspector_object_is_node(session: &v8::inspector::V8InspectorSession, object_id: &str) -> bool {
    // SAFETY: this helper is called only synchronously from the native Channel
    // callback. The Inspector handle scope that created `object_id` remains
    // active for the complete higher-ranked callback below.
    unsafe {
        session.with_unwrapped_object_in_current_handle_scope(
            v8::inspector::StringView::from(object_id.as_bytes()),
            |unwrapped| {
                let context = unwrapped.context;
                let value = unwrapped.value;
                v8::callback_scope!(unsafe let scope, context);
                let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
                    return false;
                };
                let object = v8::Local::new(scope, object);
                crate::native_bridge::object_is_node_wrapper_or_detached(scope, object)
            },
        )
    }
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finds_only_unclassified_object_handles() {
        let message = json!({
            "result": {
                "result": {
                    "type": "object",
                    "objectId": "node-object"
                },
                "properties": [
                    {
                        "value": {
                            "type": "function",
                            "objectId": "function-object"
                        }
                    },
                    {
                        "value": {
                            "type": "object",
                            "subtype": "array",
                            "objectId": "classified-object"
                        }
                    },
                    {
                        "value": {
                            "type": "string",
                            "objectId": "not-an-object"
                        }
                    }
                ]
            }
        });

        let mut objects = unclassified_remote_objects(&message);
        objects.sort();
        assert_eq!(
            objects,
            vec![("/result/result".to_owned(), "node-object".to_owned())]
        );
    }

    #[test]
    fn object_scan_respects_protocol_depth_cap() {
        let mut message = json!({
            "type": "object",
            "objectId": "too-deep"
        });
        for _ in 0..=MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH {
            message = json!({ "child": message });
        }

        assert!(unclassified_remote_objects(&message).is_empty());
    }
}

use moli_core::page::{
    DocumentNodeObjectSnapshot, DocumentNodeSnapshot, MAX_DOM_OUTPUT_TREE_DEPTH,
    MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH, RendererDomBidiNodeBindingResolution,
};
use serde_json::{Map, Value, json};

use crate::conn::{CdpConnection, CommandOwnerScope};
use crate::devtools_runtime::{
    DevToolsError, DevToolsErrorKind, DevToolsRemoteHandleId, DevToolsRemoteValue,
    DevToolsSerializationOptions, webdriver_bidi_node_shared_id_for_backend_node_id,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BidiIncludeShadowTree {
    None,
    Open,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BidiNodeSerializationOptions {
    pub(super) value_depth: i32,
    pub(super) snapshot_depth: i32,
    pub(super) include_shadow_tree: BidiIncludeShadowTree,
}

pub(super) async fn bidi_node_snapshot_for_shared_id_async(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    shared_id: &str,
    depth: i32,
    pierce: bool,
) -> Result<Option<DocumentNodeObjectSnapshot>, DevToolsError> {
    match conn
        .document_bidi_node_binding_for_owner_async(owner, shared_id)
        .await
        .map_err(|message| DevToolsError::new(DevToolsErrorKind::Internal, message))?
    {
        RendererDomBidiNodeBindingResolution::BackendNodeId(backend_node_id) => {
            return conn
                .document_node_snapshot_for_backend_node_id_for_owner_async(
                    owner,
                    backend_node_id,
                    depth,
                    pierce,
                )
                .await
                .map_err(|message| DevToolsError::new(DevToolsErrorKind::Internal, message));
        }
        RendererDomBidiNodeBindingResolution::NotFound => {}
    }
    conn.document_node_snapshot_for_runtime_remote_object_id_for_owner_async(
        owner, shared_id, depth, pierce,
    )
    .await
    .map_err(|message| DevToolsError::new(DevToolsErrorKind::Internal, message))
}

pub(super) fn bidi_node_shared_id_for_snapshot(
    snapshot: &DocumentNodeSnapshot,
) -> Option<DevToolsRemoteHandleId> {
    snapshot
        .backend_node_id
        .map(webdriver_bidi_node_shared_id_for_backend_node_id)
}

pub(super) fn bidi_node_serialization_options(
    serialization_options: Option<&DevToolsSerializationOptions>,
) -> BidiNodeSerializationOptions {
    let value_depth = match serialization_options {
        Some(options) => options
            .max_dom_depth
            .map(|depth| depth.min(i32::MAX as u64) as i32)
            .unwrap_or(-1),
        None => 0,
    };
    let snapshot_depth = if value_depth < 0 {
        -1
    } else {
        value_depth.saturating_add(1)
    };
    let include_shadow_tree =
        match serialization_options.and_then(|options| options.include_shadow_tree.as_deref()) {
            Some("open") => BidiIncludeShadowTree::Open,
            Some("all") => BidiIncludeShadowTree::All,
            _ => BidiIncludeShadowTree::None,
        };
    BidiNodeSerializationOptions {
        value_depth,
        snapshot_depth,
        include_shadow_tree,
    }
}

pub(super) fn devtools_serialization_options_for_node_probe(
    node_options: &BidiNodeSerializationOptions,
) -> DevToolsSerializationOptions {
    DevToolsSerializationOptions {
        max_object_depth: Some(0),
        max_dom_depth: (node_options.value_depth >= 0).then_some(node_options.value_depth as u64),
        include_shadow_tree: bidi_include_shadow_tree_serialization_value(node_options)
            .map(str::to_owned),
    }
}

fn bidi_include_shadow_tree_serialization_value(
    node_options: &BidiNodeSerializationOptions,
) -> Option<&'static str> {
    match node_options.include_shadow_tree {
        BidiIncludeShadowTree::None => None,
        BidiIncludeShadowTree::Open => Some("open"),
        BidiIncludeShadowTree::All => Some("all"),
    }
}

pub(super) fn bidi_node_remote_value_from_snapshot(
    snapshot: &DocumentNodeSnapshot,
    shared_id: DevToolsRemoteHandleId,
    options: &BidiNodeSerializationOptions,
) -> Value {
    bidi_node_remote_value_from_snapshot_with_budget(
        snapshot,
        shared_id,
        options,
        MAX_DOM_OUTPUT_TREE_DEPTH,
    )
}

fn bidi_node_remote_value_from_snapshot_with_budget(
    snapshot: &DocumentNodeSnapshot,
    shared_id: DevToolsRemoteHandleId,
    options: &BidiNodeSerializationOptions,
    remaining_tree_depth: usize,
) -> Value {
    build_bidi_node_remote_value_from_snapshot(
        snapshot,
        shared_id.into_string(),
        *options,
        remaining_tree_depth,
    )
}

pub(super) fn bidi_node_value_from_snapshot(
    snapshot: &DocumentNodeSnapshot,
    options: &BidiNodeSerializationOptions,
) -> Value {
    bidi_node_value_from_snapshot_with_budget(
        snapshot,
        options,
        MAX_DOM_OUTPUT_TREE_DEPTH.saturating_sub(1),
    )
}

fn bidi_node_value_from_snapshot_with_budget(
    snapshot: &DocumentNodeSnapshot,
    options: &BidiNodeSerializationOptions,
    remaining_tree_depth: usize,
) -> Value {
    build_bidi_node_value_from_snapshot(snapshot, *options, remaining_tree_depth)
}

enum BidiSnapshotFrame<'a> {
    EnterRemote {
        snapshot: &'a DocumentNodeSnapshot,
        shared_id: String,
        options: BidiNodeSerializationOptions,
        remaining_tree_depth: usize,
        result_slot: usize,
    },
    FinishRemote {
        shared_id: String,
        value_slot: usize,
        result_slot: usize,
    },
    EnterValue {
        snapshot: &'a DocumentNodeSnapshot,
        options: BidiNodeSerializationOptions,
        remaining_tree_depth: usize,
        result_slot: usize,
    },
    FinishValue {
        object: Map<String, Value>,
        shadow_root_slot: Option<usize>,
        include_shadow_root: bool,
        child_slots: Option<Vec<usize>>,
        result_slot: usize,
    },
}

fn build_bidi_node_remote_value_from_snapshot(
    snapshot: &DocumentNodeSnapshot,
    shared_id: String,
    options: BidiNodeSerializationOptions,
    remaining_tree_depth: usize,
) -> Value {
    let mut slots = Vec::new();
    let result_slot = push_bidi_snapshot_value_slot(&mut slots);
    let mut stack = vec![BidiSnapshotFrame::EnterRemote {
        snapshot,
        shared_id,
        options,
        remaining_tree_depth,
        result_slot,
    }];
    run_bidi_snapshot_frames(&mut slots, &mut stack);
    slots[result_slot].take().unwrap_or(Value::Null)
}

fn build_bidi_node_value_from_snapshot(
    snapshot: &DocumentNodeSnapshot,
    options: BidiNodeSerializationOptions,
    remaining_tree_depth: usize,
) -> Value {
    let mut slots = Vec::new();
    let result_slot = push_bidi_snapshot_value_slot(&mut slots);
    let mut stack = vec![BidiSnapshotFrame::EnterValue {
        snapshot,
        options,
        remaining_tree_depth,
        result_slot,
    }];
    run_bidi_snapshot_frames(&mut slots, &mut stack);
    slots[result_slot].take().unwrap_or(Value::Null)
}

fn run_bidi_snapshot_frames<'a>(
    slots: &mut Vec<Option<Value>>,
    stack: &mut Vec<BidiSnapshotFrame<'a>>,
) {
    while let Some(frame) = stack.pop() {
        match frame {
            BidiSnapshotFrame::EnterRemote {
                snapshot,
                shared_id,
                options,
                remaining_tree_depth,
                result_slot,
            } => {
                let value_slot = push_bidi_snapshot_value_slot(slots);
                stack.push(BidiSnapshotFrame::FinishRemote {
                    shared_id,
                    value_slot,
                    result_slot,
                });
                stack.push(BidiSnapshotFrame::EnterValue {
                    snapshot,
                    options,
                    remaining_tree_depth: remaining_tree_depth.saturating_sub(1),
                    result_slot: value_slot,
                });
            }
            BidiSnapshotFrame::FinishRemote {
                shared_id,
                value_slot,
                result_slot,
            } => {
                let value = slots[value_slot].take().unwrap_or(Value::Null);
                slots[result_slot] = Some(json!({
                    "type": "node",
                    "sharedId": shared_id,
                    "value": value,
                }));
            }
            BidiSnapshotFrame::EnterValue {
                snapshot,
                options,
                remaining_tree_depth,
                result_slot,
            } => {
                enter_bidi_snapshot_value_frame(
                    slots,
                    stack,
                    snapshot,
                    options,
                    remaining_tree_depth,
                    result_slot,
                );
            }
            BidiSnapshotFrame::FinishValue {
                mut object,
                shadow_root_slot,
                include_shadow_root,
                child_slots,
                result_slot,
            } => {
                if include_shadow_root {
                    let shadow_root = shadow_root_slot
                        .and_then(|slot| slots[slot].take())
                        .unwrap_or(Value::Null);
                    object.insert("shadowRoot".to_owned(), shadow_root);
                }
                if let Some(child_slots) = child_slots {
                    let children = child_slots
                        .into_iter()
                        .filter_map(|slot| slots[slot].take())
                        .collect();
                    object.insert("children".to_owned(), Value::Array(children));
                }
                slots[result_slot] = Some(Value::Object(object));
            }
        }
    }
}

fn enter_bidi_snapshot_value_frame<'a>(
    slots: &mut Vec<Option<Value>>,
    stack: &mut Vec<BidiSnapshotFrame<'a>>,
    snapshot: &'a DocumentNodeSnapshot,
    options: BidiNodeSerializationOptions,
    remaining_tree_depth: usize,
    result_slot: usize,
) {
    let mut value = serde_json::Map::new();
    value.insert("nodeType".to_owned(), json!(snapshot.node_type));
    value.insert("childNodeCount".to_owned(), json!(snapshot.child_count));

    let mut include_shadow_root = false;
    let mut shadow_root_slot = None;
    let mut pending_frames = Vec::new();

    match snapshot.node_type {
        1 => {
            value.insert("localName".to_owned(), json!(snapshot.local_name));
            value.insert(
                "namespaceURI".to_owned(),
                snapshot
                    .namespace_uri
                    .as_ref()
                    .filter(|namespace_uri| !namespace_uri.is_empty())
                    .map_or(Value::Null, |namespace_uri| json!(namespace_uri)),
            );

            let attributes = snapshot
                .attributes
                .iter()
                .map(|attribute| (attribute.local_name.clone(), json!(attribute.value)))
                .collect();
            value.insert("attributes".to_owned(), Value::Object(attributes));
            include_shadow_root = true;
            if remaining_tree_depth > 0
                && let Some(shadow_root) = snapshot.shadow_roots.first()
                && let Some(shared_id) = bidi_node_shared_id_for_snapshot(shadow_root)
            {
                let slot = push_bidi_snapshot_value_slot(slots);
                shadow_root_slot = Some(slot);
                pending_frames.push(BidiSnapshotFrame::EnterRemote {
                    snapshot: shadow_root,
                    shared_id: shared_id.into_string(),
                    options: options.with_shadow_root_child_depth(),
                    remaining_tree_depth,
                    result_slot: slot,
                });
            }
        }
        3 | 4 | 7 | 8 => {
            value.insert("nodeValue".to_owned(), json!(snapshot.node_value));
        }
        11 if snapshot.shadow_root_type.is_some() => {
            if let Some(mode) = snapshot.shadow_root_type.as_deref() {
                value.insert("mode".to_owned(), json!(mode));
            }
        }
        _ => {}
    }

    let mut child_slots = None;
    if remaining_tree_depth > 0
        && bidi_node_snapshot_children_should_be_serialized(snapshot, &options)
    {
        let mut slots_for_children = Vec::new();
        for child in &snapshot.children {
            let Some(shared_id) = bidi_node_shared_id_for_snapshot(child) else {
                continue;
            };
            let slot = push_bidi_snapshot_value_slot(slots);
            slots_for_children.push(slot);
            pending_frames.push(BidiSnapshotFrame::EnterRemote {
                snapshot: child,
                shared_id: shared_id.into_string(),
                options: options.with_decremented_value_depth(),
                remaining_tree_depth,
                result_slot: slot,
            });
        }
        child_slots = Some(slots_for_children);
    }

    stack.push(BidiSnapshotFrame::FinishValue {
        object: value,
        shadow_root_slot,
        include_shadow_root,
        child_slots,
        result_slot,
    });
    for frame in pending_frames.into_iter().rev() {
        stack.push(frame);
    }
}

fn push_bidi_snapshot_value_slot(slots: &mut Vec<Option<Value>>) -> usize {
    let slot = slots.len();
    slots.push(None);
    slot
}

impl BidiNodeSerializationOptions {
    pub(super) fn with_decremented_value_depth(self) -> Self {
        let value_depth = if self.value_depth < 0 {
            self.value_depth
        } else {
            self.value_depth.saturating_sub(1)
        };
        Self {
            value_depth,
            snapshot_depth: self.snapshot_depth,
            include_shadow_tree: self.include_shadow_tree,
        }
    }

    fn with_shadow_root_child_depth(self) -> Self {
        Self {
            snapshot_depth: self.snapshot_depth,
            ..self
        }
    }
}

fn bidi_node_snapshot_children_should_be_serialized(
    snapshot: &DocumentNodeSnapshot,
    options: &BidiNodeSerializationOptions,
) -> bool {
    if snapshot.shadow_root_type.is_some() {
        let Some(mode) = snapshot.shadow_root_type.as_deref() else {
            return false;
        };
        let shadow_tree_included = match options.include_shadow_tree {
            BidiIncludeShadowTree::None => false,
            BidiIncludeShadowTree::Open => mode == "open",
            BidiIncludeShadowTree::All => true,
        };
        return shadow_tree_included && options.value_depth != 0;
    }
    options.value_depth != 0
}

pub(super) fn bidi_node_remote_value_from_deep_serialized_remote_value(
    remote_value: &DevToolsRemoteValue,
) -> Option<Value> {
    let deep_serialized_value = remote_value.deep_serialized_value.as_ref()?;
    let existing_shared_id = remote_value
        .shared_id
        .as_ref()
        .map(|shared_id| shared_id.as_str());
    bidi_node_remote_value_from_deep_serialized_node(
        deep_serialized_value,
        existing_shared_id,
        MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH,
    )
}

fn bidi_node_remote_value_from_deep_serialized_node(
    value: &Value,
    existing_shared_id: Option<&str>,
    remaining_tree_depth: usize,
) -> Option<Value> {
    build_bidi_node_remote_value_from_deep_serialized_node(
        value.clone(),
        existing_shared_id.map(str::to_owned),
        remaining_tree_depth,
    )
}

fn bidi_node_shared_id_from_deep_serialized_node_value(
    node_value: &Value,
    existing_shared_id: Option<&str>,
) -> Option<String> {
    let object = node_value.as_object()?;
    object
        .get("sharedId")
        .and_then(Value::as_str)
        .or(existing_shared_id)
        .map(str::to_owned)
}

enum BidiDeepSerializedFrame {
    EnterRemote {
        value: Value,
        existing_shared_id: Option<String>,
        remaining_tree_depth: usize,
        result_slot: usize,
    },
    FinishRemote {
        shared_id: String,
        value_slot: usize,
        result_slot: usize,
    },
    EnterValue {
        node_value: Value,
        remaining_tree_depth: usize,
        result_slot: usize,
    },
    FinishValue {
        node_value: Value,
        child_slots: Option<Vec<usize>>,
        shadow_root_slot: Option<usize>,
        result_slot: usize,
    },
}

fn build_bidi_node_remote_value_from_deep_serialized_node(
    value: Value,
    existing_shared_id: Option<String>,
    remaining_tree_depth: usize,
) -> Option<Value> {
    let mut slots = Vec::new();
    let result_slot = push_bidi_deep_serialized_value_slot(&mut slots);
    let mut stack = vec![BidiDeepSerializedFrame::EnterRemote {
        value,
        existing_shared_id,
        remaining_tree_depth,
        result_slot,
    }];
    run_bidi_deep_serialized_frames(&mut slots, &mut stack);
    slots[result_slot].take()
}

fn run_bidi_deep_serialized_frames(
    slots: &mut Vec<Option<Value>>,
    stack: &mut Vec<BidiDeepSerializedFrame>,
) {
    while let Some(frame) = stack.pop() {
        match frame {
            BidiDeepSerializedFrame::EnterRemote {
                value,
                existing_shared_id,
                remaining_tree_depth,
                result_slot,
            } => {
                let Some(child_tree_depth) = remaining_tree_depth.checked_sub(1) else {
                    slots[result_slot] = None;
                    continue;
                };
                let Some(object) = value.as_object() else {
                    slots[result_slot] = None;
                    continue;
                };
                if object.get("type").and_then(Value::as_str) != Some("node") {
                    slots[result_slot] = None;
                    continue;
                }
                let Some(node_value) = object.get("value") else {
                    slots[result_slot] = None;
                    continue;
                };
                let Some(shared_id) = bidi_node_shared_id_from_deep_serialized_node_value(
                    node_value,
                    existing_shared_id.as_deref(),
                ) else {
                    slots[result_slot] = None;
                    continue;
                };
                let value_slot = push_bidi_deep_serialized_value_slot(slots);
                stack.push(BidiDeepSerializedFrame::FinishRemote {
                    shared_id,
                    value_slot,
                    result_slot,
                });
                stack.push(BidiDeepSerializedFrame::EnterValue {
                    node_value: node_value.clone(),
                    remaining_tree_depth: child_tree_depth,
                    result_slot: value_slot,
                });
            }
            BidiDeepSerializedFrame::FinishRemote {
                shared_id,
                value_slot,
                result_slot,
            } => {
                let Some(value) = slots[value_slot].take() else {
                    slots[result_slot] = None;
                    continue;
                };
                slots[result_slot] = Some(json!({
                    "type": "node",
                    "sharedId": shared_id,
                    "value": value,
                }));
            }
            BidiDeepSerializedFrame::EnterValue {
                node_value,
                remaining_tree_depth,
                result_slot,
            } => enter_bidi_deep_serialized_value_frame(
                slots,
                stack,
                node_value,
                remaining_tree_depth,
                result_slot,
            ),
            BidiDeepSerializedFrame::FinishValue {
                mut node_value,
                child_slots,
                shadow_root_slot,
                result_slot,
            } => {
                let Some(object) = node_value.as_object_mut() else {
                    slots[result_slot] = None;
                    continue;
                };
                if let Some(child_slots) = child_slots {
                    let children = child_slots
                        .into_iter()
                        .filter_map(|slot| slots[slot].take())
                        .collect();
                    object.insert("children".to_owned(), Value::Array(children));
                }
                if let Some(slot) = shadow_root_slot {
                    let Some(shadow_root) = slots[slot].take() else {
                        slots[result_slot] = None;
                        continue;
                    };
                    object.insert("shadowRoot".to_owned(), shadow_root);
                }
                slots[result_slot] = Some(node_value);
            }
        }
    }
}

fn enter_bidi_deep_serialized_value_frame(
    slots: &mut Vec<Option<Value>>,
    stack: &mut Vec<BidiDeepSerializedFrame>,
    mut node_value: Value,
    remaining_tree_depth: usize,
    result_slot: usize,
) {
    let Some(object) = node_value.as_object_mut() else {
        slots[result_slot] = None;
        return;
    };
    object.remove("backendNodeId");
    object.remove("loaderId");
    if object
        .get("namespaceURI")
        .and_then(Value::as_str)
        .is_some_and(str::is_empty)
    {
        object.insert("namespaceURI".to_owned(), Value::Null);
    }

    let mut child_slots = None;
    let mut shadow_root_slot = None;
    let mut pending_frames = Vec::new();

    if let Some(children) = object.get_mut("children").and_then(Value::as_array_mut) {
        if remaining_tree_depth > 0 {
            let raw_children = std::mem::take(children);
            let mut slots_for_children = Vec::new();
            for child in raw_children {
                let slot = push_bidi_deep_serialized_value_slot(slots);
                slots_for_children.push(slot);
                pending_frames.push(BidiDeepSerializedFrame::EnterRemote {
                    value: child,
                    existing_shared_id: None,
                    remaining_tree_depth,
                    result_slot: slot,
                });
            }
            child_slots = Some(slots_for_children);
        } else {
            children.clear();
        }
    }

    if let Some(shadow_root) = object.get_mut("shadowRoot")
        && !shadow_root.is_null()
    {
        if remaining_tree_depth == 0 {
            *shadow_root = Value::Null;
        } else {
            let slot = push_bidi_deep_serialized_value_slot(slots);
            shadow_root_slot = Some(slot);
            pending_frames.push(BidiDeepSerializedFrame::EnterRemote {
                value: std::mem::take(shadow_root),
                existing_shared_id: None,
                remaining_tree_depth,
                result_slot: slot,
            });
        }
    }

    stack.push(BidiDeepSerializedFrame::FinishValue {
        node_value,
        child_slots,
        shadow_root_slot,
        result_slot,
    });
    for frame in pending_frames.into_iter().rev() {
        stack.push(frame);
    }
}

fn push_bidi_deep_serialized_value_slot(slots: &mut Vec<Option<Value>>) -> usize {
    let slot = slots.len();
    slots.push(None);
    slot
}

pub(super) fn bidi_node_remote_value_shared_id(remote: &Value) -> Option<&str> {
    remote.get("sharedId").and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use moli_core::page::RENDERER_BACKEND_NODE_ID_START;
    use moli_page_types::{DocumentNodeSnapshot, DocumentSnapshotNodeId};
    use serde_json::{Value, json};

    use crate::devtools_runtime::{DevToolsRemoteHandleId, DevToolsRemoteValue};

    use super::{
        BidiIncludeShadowTree, BidiNodeSerializationOptions,
        bidi_node_remote_value_from_deep_serialized_node,
        bidi_node_remote_value_from_deep_serialized_remote_value,
        bidi_node_remote_value_from_snapshot_with_budget,
    };

    fn deep_serialized_node_remote(value: Value) -> DevToolsRemoteValue {
        DevToolsRemoteValue {
            value: Value::Null,
            handle: None,
            shared_id: Some(DevToolsRemoteHandleId::from("REMOTE-NODE")),
            node_id: None,
            backend_node_id: None,
            window_context: None,
            realm: None,
            remote_type: Some("object".to_owned()),
            remote_subtype: Some("node".to_owned()),
            unserializable_value: None,
            description: None,
            class_name: None,
            deep_serialized_value: Some(value),
            node_value: None,
        }
    }

    fn element_snapshot(index: usize, child: Option<DocumentNodeSnapshot>) -> DocumentNodeSnapshot {
        let children = child.into_iter().collect::<Vec<_>>();
        DocumentNodeSnapshot {
            node_id: DocumentSnapshotNodeId::new(index),
            parent_id: None,
            inspector_identity: None,
            inspector_parent_identity: None,
            frontend_node_id: None,
            parent_frontend_node_id: None,
            backend_node_id: Some(RENDERER_BACKEND_NODE_ID_START + index as u32),
            frame_id: None,
            node_type: 1,
            node_name: "DIV".to_owned(),
            local_name: "div".to_owned(),
            node_value: String::new(),
            child_count: children.len(),
            document_url: "about:blank".to_owned(),
            base_url: "about:blank".to_owned(),
            namespace_uri: Some("http://www.w3.org/1999/xhtml".to_owned()),
            attributes: Vec::new(),
            document_type_name: None,
            public_id: None,
            system_id: None,
            is_element: true,
            has_geometry: false,
            shadow_root_type: None,
            shadow_roots: Vec::new(),
            pseudo_type: None,
            pseudo_elements: Vec::new(),
            associated: None,
            children,
        }
    }

    #[test]
    fn deep_serialized_node_value_uses_explicit_shared_ids() {
        let remote = deep_serialized_node_remote(json!({
            "type": "node",
            "value": {
                "backendNodeId": 8,
                "loaderId": "FRAME-1",
                "sharedId": "ROOT-NODE",
                "nodeType": 1,
                "childNodeCount": 1,
                "localName": "div",
                "namespaceURI": "",
                "attributes": {},
                "children": [
                    {
                        "type": "node",
                        "value": {
                            "backendNodeId": 9,
                            "loaderId": "FRAME-1",
                            "sharedId": "CHILD-NODE",
                            "nodeType": 3,
                            "childNodeCount": 0,
                            "nodeValue": "hello"
                        }
                    }
                ],
                "shadowRoot": null
            }
        }));

        let value = bidi_node_remote_value_from_deep_serialized_remote_value(&remote)
            .expect("deep serialized node should map directly");

        assert_eq!(value["type"], json!("node"));
        assert_eq!(value["sharedId"], json!("ROOT-NODE"));
        assert_eq!(value["value"]["namespaceURI"], Value::Null);
        assert!(value["value"].get("backendNodeId").is_none());
        assert!(value["value"].get("loaderId").is_none());
        assert_eq!(
            value["value"]["children"][0]["sharedId"],
            json!("CHILD-NODE")
        );
        assert!(
            value["value"]["children"][0]["value"]
                .get("backendNodeId")
                .is_none()
        );
        assert!(
            value["value"]["children"][0]["value"]
                .get("loaderId")
                .is_none()
        );
    }

    #[test]
    fn deep_serialized_node_value_does_not_decode_backend_node_id_as_shared_id() {
        let remote = deep_serialized_node_remote(json!({
            "type": "node",
            "value": {
                "backendNodeId": 8,
                "loaderId": "FRAME-1",
                "nodeType": 1,
                "childNodeCount": 0,
                "localName": "div",
                "namespaceURI": "",
                "attributes": {},
                "children": [],
                "shadowRoot": null
            }
        }));

        let value = bidi_node_remote_value_from_deep_serialized_remote_value(&remote)
            .expect("existing remote id can still identify the node remote");

        assert_eq!(value["sharedId"], json!("REMOTE-NODE"));
        assert!(value["value"].get("backendNodeId").is_none());
        assert!(value["value"].get("loaderId").is_none());
    }

    #[test]
    fn deep_serialized_node_conversion_respects_recursion_budget() {
        let value = json!({
            "type": "node",
            "value": {
                "backendNodeId": 8,
                "loaderId": "FRAME-1",
                "sharedId": "ROOT-NODE",
                "nodeType": 1,
                "childNodeCount": 1,
                "localName": "div",
                "namespaceURI": "",
                "attributes": {},
                "children": [
                    {
                        "type": "node",
                        "value": {
                            "backendNodeId": 9,
                            "loaderId": "FRAME-1",
                            "sharedId": "CHILD-NODE",
                            "nodeType": 3,
                            "childNodeCount": 0,
                            "nodeValue": "hello"
                        }
                    }
                ],
                "shadowRoot": null
            }
        });

        let value = bidi_node_remote_value_from_deep_serialized_node(&value, None, 1)
            .expect("root node should still serialize at the structural limit");

        assert_eq!(value["sharedId"], json!("ROOT-NODE"));
        assert!(value["value"].get("backendNodeId").is_none());
        assert!(value["value"].get("loaderId").is_none());
        assert_eq!(value["value"]["children"], json!([]));
    }

    #[test]
    fn snapshot_node_conversion_respects_structural_budget_with_unbounded_value_depth() {
        let snapshot = element_snapshot(7, Some(element_snapshot(8, None)));
        let options = BidiNodeSerializationOptions {
            value_depth: -1,
            snapshot_depth: -1,
            include_shadow_tree: BidiIncludeShadowTree::All,
        };

        let value = bidi_node_remote_value_from_snapshot_with_budget(
            &snapshot,
            DevToolsRemoteHandleId::from("ROOT-NODE"),
            &options,
            1,
        );

        assert_eq!(value["sharedId"], json!("ROOT-NODE"));
        assert!(value["value"].get("children").is_none());

        let value = bidi_node_remote_value_from_snapshot_with_budget(
            &snapshot,
            DevToolsRemoteHandleId::from("ROOT-NODE"),
            &options,
            2,
        );

        assert_eq!(
            value["value"]["children"][0]["sharedId"],
            json!("moli:bidi-renderer-node:2000000008")
        );
    }

    #[test]
    fn snapshot_node_conversion_does_not_forge_shared_ids_without_backend_identity() {
        let mut child = element_snapshot(8, None);
        child.backend_node_id = None;
        let snapshot = element_snapshot(7, Some(child));
        let options = BidiNodeSerializationOptions {
            value_depth: -1,
            snapshot_depth: -1,
            include_shadow_tree: BidiIncludeShadowTree::All,
        };

        let value = bidi_node_remote_value_from_snapshot_with_budget(
            &snapshot,
            DevToolsRemoteHandleId::from("ROOT-NODE"),
            &options,
            2,
        );

        assert_eq!(value["sharedId"], json!("ROOT-NODE"));
        assert_eq!(value["value"]["children"], json!([]));
    }
}

use std::collections::HashMap;

use anyhow::Result;
use moli_core::page::{ChildFrameTreeSnapshot, Page};
use serde_json::{Value, json};

pub(super) async fn render_json(page: &mut Page, with_frames: bool) -> Result<String> {
    let payloads = collect_payloads(page, with_frames).await?;
    Ok(serde_json::to_string_pretty(&payloads)?)
}

pub(super) async fn render_text(page: &mut Page, with_frames: bool) -> Result<String> {
    let payloads = collect_payloads(page, with_frames).await?;
    Ok(render_payloads_text(&payloads))
}

async fn collect_payloads(page: &mut Page, with_frames: bool) -> Result<Vec<Value>> {
    let mut payloads = page
        .accessibility_tree_payloads_for_document_async(None)
        .await?;
    if !with_frames {
        return Ok(payloads);
    }

    let frame_tree = page.child_frame_tree_snapshot_async().await?;
    let mut frame_ids = Vec::new();
    collect_child_frame_ids(&frame_tree, &mut frame_ids);

    for frame_id in frame_ids {
        let Some(owner) = page
            .child_frame_owner_node_reference_async(&frame_id)
            .await?
        else {
            continue;
        };
        let Some(child_payloads) = page
            .child_frame_accessibility_tree_payloads_async(&frame_id, None)
            .await?
        else {
            continue;
        };
        attach_child_frame_accessibility_tree(&mut payloads, owner.backend_node_id, child_payloads);
    }

    Ok(payloads)
}

fn collect_child_frame_ids(frames: &[ChildFrameTreeSnapshot], frame_ids: &mut Vec<String>) {
    for frame in frames {
        frame_ids.push(frame.frame_id.clone());
        collect_child_frame_ids(&frame.child_frames, frame_ids);
    }
}

fn attach_child_frame_accessibility_tree(
    payloads: &mut Vec<Value>,
    owner_backend_node_id: u32,
    mut child_payloads: Vec<Value>,
) {
    let Some(child_root_id) = child_payloads
        .first()
        .and_then(|payload| payload.get("nodeId"))
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return;
    };
    let Some(owner_index) = payloads.iter().position(|payload| {
        payload.get("backendDOMNodeId").and_then(Value::as_u64)
            == Some(u64::from(owner_backend_node_id))
    }) else {
        return;
    };
    let Some(owner_node_id) = payloads[owner_index]
        .get("nodeId")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return;
    };
    let Some(child_root) = child_payloads.first_mut().and_then(Value::as_object_mut) else {
        return;
    };
    child_root.insert("parentId".to_owned(), json!(owner_node_id));

    let Some(owner) = payloads[owner_index].as_object_mut() else {
        return;
    };
    let child_ids = owner
        .entry("childIds".to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(child_ids) = child_ids.as_array_mut() else {
        return;
    };
    if !child_ids.iter().any(|child_id| child_id == &child_root_id) {
        child_ids.push(json!(child_root_id));
    }
    payloads.append(&mut child_payloads);
}

fn render_payloads_text(payloads: &[Value]) -> String {
    if payloads.is_empty() {
        return String::new();
    }

    let mut by_id = HashMap::new();
    for payload in payloads {
        if let Some(id) = payload.get("nodeId").and_then(Value::as_str) {
            by_id.insert(id.to_owned(), payload);
        }
    }

    let mut out = String::new();
    if let Some(root_id) = payloads[0].get("nodeId").and_then(Value::as_str) {
        render_node_text(root_id, &by_id, 0, &mut out);
    }
    out.trim_end().to_owned()
}

fn render_node_text(
    node_id: &str,
    by_id: &HashMap<String, &Value>,
    depth: usize,
    out: &mut String,
) {
    let Some(payload) = by_id.get(node_id) else {
        return;
    };

    let role = payload["role"]["value"].as_str().unwrap_or("Unknown");
    let name = payload["name"]["value"].as_str().unwrap_or_default();
    let value = payload["value"]["value"].as_str().unwrap_or_default();
    let backend = payload["backendDOMNodeId"].as_u64().unwrap_or(0);

    out.push_str(&"  ".repeat(depth));
    out.push_str("- ");
    out.push_str(role);
    if !name.is_empty() {
        out.push_str(": ");
        out.push_str(name);
    }
    if !value.is_empty() {
        out.push_str(" = ");
        out.push_str(value);
    }
    if backend != 0 {
        out.push_str(&format!(" [backendNodeId={backend}]"));
    }
    out.push('\n');

    for child_id in payload["childIds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        render_node_text(child_id, by_id, depth + 1, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(frame_id: &str, child_frames: Vec<ChildFrameTreeSnapshot>) -> ChildFrameTreeSnapshot {
        ChildFrameTreeSnapshot {
            frame_id: frame_id.to_owned(),
            loader_id: format!("loader-{frame_id}"),
            name: None,
            owner_element_id: None,
            url: "about:blank".to_owned(),
            storage_key: String::new(),
            security_origin_inherited: false,
            security_origin_opaque: false,
            child_frames,
        }
    }

    #[test]
    fn child_frame_ids_are_collected_in_preorder() {
        let frames = vec![
            frame("first", vec![frame("nested", Vec::new())]),
            frame("second", Vec::new()),
        ];
        let mut frame_ids = Vec::new();

        collect_child_frame_ids(&frames, &mut frame_ids);

        assert_eq!(frame_ids, ["first", "nested", "second"]);
    }

    #[test]
    fn child_tree_is_attached_to_its_iframe_owner() {
        let mut payloads = vec![
            json!({
                "nodeId": "AX-1",
                "backendDOMNodeId": 1,
                "role": { "value": "RootWebArea" },
                "childIds": ["AX-2"]
            }),
            json!({
                "nodeId": "AX-2",
                "parentId": "AX-1",
                "backendDOMNodeId": 2,
                "role": { "value": "Iframe" }
            }),
        ];
        let child_payloads = vec![
            json!({
                "nodeId": "AX-3",
                "backendDOMNodeId": 3,
                "role": { "value": "RootWebArea" },
                "childIds": ["AX-4"]
            }),
            json!({
                "nodeId": "AX-4",
                "parentId": "AX-3",
                "backendDOMNodeId": 4,
                "role": { "value": "button" },
                "name": { "value": "Child action" }
            }),
        ];

        attach_child_frame_accessibility_tree(&mut payloads, 2, child_payloads);

        assert_eq!(payloads[1]["childIds"], json!(["AX-3"]));
        assert_eq!(payloads[2]["parentId"], "AX-2");
        assert!(render_payloads_text(&payloads).contains("button: Child action"));
    }

    #[test]
    fn child_tree_without_a_matching_owner_is_not_appended() {
        let mut payloads = vec![json!({
            "nodeId": "AX-1",
            "backendDOMNodeId": 1,
            "role": { "value": "RootWebArea" }
        })];
        let original = payloads.clone();

        attach_child_frame_accessibility_tree(
            &mut payloads,
            99,
            vec![json!({
                "nodeId": "AX-2",
                "backendDOMNodeId": 2,
                "role": { "value": "RootWebArea" }
            })],
        );

        assert_eq!(payloads, original);
    }
}

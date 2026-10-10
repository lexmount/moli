use std::{collections::HashMap, fmt::Write};

use anyhow::Result;
use moli_core::page::{AccessibilityNode, AccessibilityNodeId, ChildFrameTreeSnapshot, Page};

pub(super) async fn render_json(page: &mut Page, with_frames: bool) -> Result<String> {
    let payloads = collect_payloads(page, with_frames).await?;
    Ok(serde_json::to_string_pretty(&payloads)?)
}

pub(super) async fn render_text(page: &mut Page, with_frames: bool) -> Result<String> {
    let payloads = collect_payloads(page, with_frames).await?;
    Ok(render_payloads_text(&payloads))
}

async fn collect_payloads(page: &mut Page, with_frames: bool) -> Result<Vec<AccessibilityNode>> {
    let mut payloads = page
        .accessibility_tree_nodes_async(None)
        .await?
        .unwrap_or_default();
    if !with_frames {
        return Ok(payloads);
    }

    let frame_tree = page.child_frame_tree_snapshot_async().await?;
    let mut frame_ids = Vec::new();
    collect_child_frame_ids(&frame_tree, &mut frame_ids);

    for frame_id in frame_ids {
        let Some(owner) = page
            .child_frame_owner_node_reference_async(&frame_id, None)
            .await?
        else {
            continue;
        };
        let Some(child_payloads) = page.accessibility_tree_nodes_async(Some(frame_id)).await?
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
    payloads: &mut Vec<AccessibilityNode>,
    owner_backend_node_id: u32,
    mut child_payloads: Vec<AccessibilityNode>,
) {
    let Some(child_root) = child_payloads.first_mut() else {
        return;
    };
    let Some(owner) = payloads.iter_mut().find(|node| {
        node.backend_node_id
            .is_some_and(|id| id.get() == owner_backend_node_id)
    }) else {
        return;
    };
    child_root.parent_id = Some(owner.node_id);
    let child_ids = owner.child_ids.get_or_insert_with(Vec::new);
    if !child_ids.contains(&child_root.node_id) {
        child_ids.push(child_root.node_id);
    }
    payloads.append(&mut child_payloads);
}

fn render_payloads_text(payloads: &[AccessibilityNode]) -> String {
    let Some(root) = payloads.first() else {
        return String::new();
    };
    let by_id: HashMap<AccessibilityNodeId, &AccessibilityNode> =
        payloads.iter().map(|node| (node.node_id, node)).collect();
    let mut out = String::new();
    let mut pending = vec![(root.node_id, 0)];
    while let Some((node_id, depth)) = pending.pop() {
        let Some(node) = by_id.get(&node_id) else {
            continue;
        };
        for _ in 0..depth {
            out.push_str("  ");
        }
        out.push_str("- ");
        out.push_str(node.role);
        if let Some(name) = &node.name
            && !name.is_empty()
        {
            out.push_str(": ");
            out.push_str(name);
        }
        if let Some(value) = node.value().and_then(|value| value["value"].as_str())
            && !value.is_empty()
        {
            out.push_str(" = ");
            out.push_str(value);
        }
        if let Some(backend) = node.backend_node_id {
            write!(&mut out, " [backendNodeId={backend}]").expect("writing to a String");
        }
        out.push('\n');
        pending.extend(
            node.child_ids
                .iter()
                .flatten()
                .rev()
                .map(|child| (*child, depth + 1)),
        );
    }
    out.truncate(out.trim_end().len());
    out
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use super::*;

    fn dom_id(id: u32) -> AccessibilityNodeId {
        AccessibilityNodeId::Dom(NonZeroU32::new(id).expect("fixture backend ID"))
    }

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

    fn node(
        id: u32,
        role: &'static str,
        parent: Option<u32>,
        children: &[u32],
    ) -> AccessibilityNode {
        let mut node = AccessibilityNode::new(dom_id(id), role);
        node.parent_id = parent.map(dom_id);
        node.child_ids = Some(children.iter().copied().map(dom_id).collect());
        node
    }

    #[test]
    fn child_tree_is_attached_to_its_iframe_owner() {
        let mut payloads = vec![
            node(1, "RootWebArea", None, &[2]),
            node(2, "Iframe", Some(1), &[]),
        ];
        let mut child_button = node(4, "button", Some(3), &[]);
        child_button.name = Some("Child action".to_owned());
        let child_payloads = vec![node(3, "RootWebArea", None, &[4]), child_button];

        attach_child_frame_accessibility_tree(&mut payloads, 2, child_payloads);

        assert_eq!(payloads[1].child_ids, Some(vec![dom_id(3)]));
        assert_eq!(payloads[2].parent_id, Some(dom_id(2)));
        assert!(render_payloads_text(&payloads).contains("button: Child action"));
    }

    #[test]
    fn child_tree_without_a_matching_owner_is_not_appended() {
        let mut payloads = vec![node(1, "RootWebArea", None, &[])];
        let original = serde_json::to_value(&payloads).expect("fixture JSON");
        attach_child_frame_accessibility_tree(
            &mut payloads,
            99,
            vec![node(2, "RootWebArea", None, &[])],
        );
        assert_eq!(
            serde_json::to_value(payloads).expect("fixture JSON"),
            original
        );
    }

    #[test]
    fn text_preserves_child_order_and_value_rules_without_recursive_traversal() {
        const DEPTH: u32 = 2048;
        let mut nodes = vec![node(1, "RootWebArea", None, &[2, DEPTH + 1])];
        for id in 2..=DEPTH {
            let next = [id + 1];
            nodes.push(node(
                id,
                "generic",
                Some(id - 1),
                if id < DEPTH { &next } else { &[] },
            ));
        }
        let mut last = node(DEPTH + 1, "textbox", Some(1), &[]);
        last.name = Some("Name".to_owned());
        last.set_rare_data(
            None,
            Some(serde_json::json!({"type":"string", "value":"value"})),
            Some(Vec::new()),
        );
        nodes.push(last);
        let text = render_payloads_text(&nodes);
        assert_eq!(text.lines().count(), (DEPTH + 1) as usize);
        assert!(text.ends_with("  - textbox: Name = value [backendNodeId=2049]"));
    }
}

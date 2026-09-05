use anyhow::Result;
use moli_core::page::{DocumentNodeSnapshot, Page, is_renderer_backend_node_id};
use serde_json::{Value, json};

pub async fn summarize_node_details_async(page: &mut Page, backend_node_id: u32) -> Result<Value> {
    let snapshot = document_node_snapshot_for_backend_node_id(page, backend_node_id).await?;
    let Some(snapshot) = snapshot else {
        return Ok(json!({
            "error": format!("backendNodeId `{backend_node_id}` not found")
        }));
    };
    let accessibility = accessibility_node_payload_for_backend_node_id(page, backend_node_id)
        .await?
        .unwrap_or_else(|| json!({}));
    let mut options = Vec::new();
    if snapshot.local_name == "select" {
        for child in &snapshot.children {
            if child.local_name != "option" {
                continue;
            }
            options.push(json!({
                "text": child.children.iter().find(|grandchild| grandchild.node_name == "#text").map(|text| text.node_value.clone()).unwrap_or_default(),
                "value": attribute_value(child, "value").unwrap_or_default(),
                "selected": attribute_value(child, "selected").is_some(),
            }));
        }
    }

    Ok(json!({
        "backendNodeId": backend_node_id,
        "tag": snapshot.local_name.clone(),
        "role": accessibility["role"]["value"].as_str().unwrap_or_default(),
        "name": accessibility["name"]["value"].as_str().unwrap_or_default(),
        "value": accessibility["value"]["value"].as_str().unwrap_or_default(),
        "inputType": attribute_value(&snapshot, "type").unwrap_or_default(),
        "placeholder": attribute_value(&snapshot, "placeholder").unwrap_or_default(),
        "href": attribute_value(&snapshot, "href").unwrap_or_default(),
        "checked": attribute_value(&snapshot, "checked").is_some(),
        "disabled": attribute_value(&snapshot, "disabled").is_some(),
        "options": options,
    }))
}

async fn accessibility_node_payload_for_backend_node_id(
    page: &mut Page,
    backend_node_id: u32,
) -> Result<Option<Value>> {
    if backend_node_id == 0 || !is_renderer_backend_node_id(backend_node_id) {
        return Ok(None);
    }
    let pending = page.start_accessibility_node_payload_for_backend_node_id(backend_node_id)?;
    let completion = pending.wait().await?;
    Ok(page
        .finish_accessibility_payloads_for_backend_node_id(completion)?
        .and_then(|payloads| payloads.payloads)
        .and_then(|payloads| payloads.into_iter().next()))
}

async fn document_node_snapshot_for_backend_node_id(
    page: &mut Page,
    backend_node_id: u32,
) -> Result<Option<DocumentNodeSnapshot>> {
    const NODE_DETAILS_SNAPSHOT_DEPTH: i32 = 2;

    if backend_node_id == 0 {
        return Ok(None);
    }

    if !is_renderer_backend_node_id(backend_node_id) {
        return Ok(None);
    }

    let snapshot = page
        .document_node_snapshot_for_backend_node_id_async(
            backend_node_id,
            NODE_DETAILS_SNAPSHOT_DEPTH,
            false,
        )
        .await?;
    Ok(snapshot.map(|snapshot| snapshot.snapshot))
}

fn attribute_value(snapshot: &DocumentNodeSnapshot, name: &str) -> Option<String> {
    snapshot
        .attributes
        .iter()
        .find(|attribute| attribute.local_name == name)
        .map(|attribute| attribute.value.clone())
}

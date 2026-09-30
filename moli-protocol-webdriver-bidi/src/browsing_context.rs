use moli_protocol::automation::{
    DevToolsBrowserContextId, DevToolsGetFrameTreeResult, DevToolsTargetId, DevToolsTargetInfo,
    DevToolsTargetKind, TargetLifecycleEvent,
};
use serde_json::{Value, json};

use crate::user_context::bidi_user_context_from_browser_context_id;

pub(crate) fn bidi_browsing_context_info(info: DevToolsTargetInfo) -> Option<Value> {
    bidi_browsing_context_info_from_target_info(&info, json!([]), Some(Value::Null))
}

fn bidi_browsing_context_info_from_target_info(
    info: &DevToolsTargetInfo,
    children: Value,
    parent: Option<Value>,
) -> Option<Value> {
    if !is_bidi_browsing_context_kind(info.kind) {
        return None;
    }
    let target_id = info.target_id.as_ref()?;
    Some(bidi_browsing_context_info_from_parts(
        target_id.as_str(),
        &info.url,
        info.opener_id.as_ref().map(DevToolsTargetId::as_str),
        info.browser_context_id
            .as_ref()
            .map(DevToolsBrowserContextId::as_str),
        None,
        children,
        parent,
    ))
}

pub(crate) fn bidi_browsing_context_infos_from_frame_tree_result(
    result: &DevToolsGetFrameTreeResult,
) -> Vec<Value> {
    let Some(info) = bidi_browsing_context_info_from_frame_tree(
        &result.frame_tree,
        result.max_depth,
        result
            .target_info
            .as_ref()
            .and_then(|info| info.target_id.as_ref())
            .map(DevToolsTargetId::as_str),
        result
            .target_info
            .as_ref()
            .and_then(|info| info.opener_id.as_ref())
            .map(DevToolsTargetId::as_str),
        result
            .target_info
            .as_ref()
            .and_then(|info| info.browser_context_id.as_ref())
            .map(DevToolsBrowserContextId::as_str),
        Some(Value::Null),
    ) else {
        return Vec::new();
    };
    vec![info]
}

pub(crate) fn bidi_browsing_context_info_from_cdp_target_info(
    target_info: &Value,
    children: Value,
) -> Option<Value> {
    if !is_bidi_browsing_context_cdp_type(target_info.get("type").and_then(Value::as_str)) {
        return None;
    }
    let target_id = target_info.get("targetId")?.as_str()?;
    Some(bidi_browsing_context_info_from_parts(
        target_id,
        target_info
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        target_info.get("openerId").and_then(Value::as_str),
        target_info.get("browserContextId").and_then(Value::as_str),
        None,
        children,
        Some(Value::Null),
    ))
}

pub(crate) fn bidi_browsing_context_info_from_target_lifecycle(
    event: &TargetLifecycleEvent,
    children: Value,
) -> Option<Value> {
    if let Some(target_info) = event.target_info.as_ref() {
        return bidi_browsing_context_info_from_target_info(
            target_info,
            children,
            Some(Value::Null),
        );
    }
    if !is_bidi_browsing_context_kind(event.kind) {
        return None;
    }
    Some(bidi_browsing_context_info_from_parts(
        event.target_id.as_str(),
        &event.url,
        None,
        event
            .browser_context_id
            .as_ref()
            .map(DevToolsBrowserContextId::as_str),
        None,
        children,
        Some(Value::Null),
    ))
}

fn bidi_browsing_context_info_from_frame_tree(
    frame_tree: &Value,
    depth_remaining: Option<u32>,
    client_window: Option<&str>,
    opener_id: Option<&str>,
    browser_context_id: Option<&str>,
    parent: Option<Value>,
) -> Option<Value> {
    let frame = frame_tree.get("frame")?;
    let frame_id = frame.get("id")?.as_str()?;
    let client_window = client_window.unwrap_or(frame_id);
    let children = match depth_remaining {
        Some(0) => Value::Null,
        _ => Value::Array(
            frame_tree
                .get("childFrames")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|child| {
                    bidi_browsing_context_info_from_frame_tree(
                        child,
                        depth_remaining.map(|depth| depth.saturating_sub(1)),
                        Some(client_window),
                        None,
                        browser_context_id,
                        None,
                    )
                })
                .collect(),
        ),
    };
    Some(bidi_browsing_context_info_from_parts(
        frame_id,
        frame.get("url").and_then(Value::as_str).unwrap_or_default(),
        opener_id,
        browser_context_id,
        Some(client_window),
        children,
        parent,
    ))
}

fn is_bidi_browsing_context_kind(kind: DevToolsTargetKind) -> bool {
    matches!(
        kind,
        DevToolsTargetKind::Page
            | DevToolsTargetKind::Frame
            | DevToolsTargetKind::SharedWorker
            | DevToolsTargetKind::ServiceWorker
    )
}

fn is_bidi_browsing_context_cdp_type(target_type: Option<&str>) -> bool {
    matches!(
        target_type,
        Some("page" | "iframe" | "shared_worker" | "service_worker")
    )
}

fn bidi_browsing_context_info_from_parts(
    target_id: &str,
    url: &str,
    opener_id: Option<&str>,
    browser_context_id: Option<&str>,
    client_window: Option<&str>,
    children: Value,
    parent: Option<Value>,
) -> Value {
    let user_context = bidi_user_context_from_browser_context_id(browser_context_id);
    let mut payload = json!({
        "children": children,
        "clientWindow": client_window.unwrap_or(target_id),
        "context": target_id,
        "originalOpener": opener_id.map(Value::from).unwrap_or(Value::Null),
        "url": url,
        "userContext": user_context,
    });
    if let Some(parent) = parent
        && let Some(payload) = payload.as_object_mut()
    {
        payload.insert("parent".to_owned(), parent);
    }
    payload
}

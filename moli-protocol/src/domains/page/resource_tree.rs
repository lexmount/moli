use super::*;

pub(super) fn get_frame_tree_command_output_plan(
    output_kind: FrameTreeCommandOutputKind,
    target_id: String,
    target_loader_id: String,
    target_url: String,
    target_unreachable_url: Option<String>,
    target_security_origin: String,
    target_secure_context_type: String,
    target_mime_type: String,
    child_frame_snapshots: Vec<ChildFrameTreeSnapshot>,
    resource_records: &[moli_core::page::SubresourceNetworkRecord],
) -> CommandOutputPlan {
    let frame_tree = frame_tree_payload(
        target_id,
        target_loader_id,
        target_url,
        target_unreachable_url,
        target_security_origin,
        target_secure_context_type,
        target_mime_type,
        child_frame_snapshots,
    );
    let frame_tree = match output_kind {
        FrameTreeCommandOutputKind::FrameTree => frame_tree,
        FrameTreeCommandOutputKind::ResourceTree => {
            resource_tree::attach_frame_resources(frame_tree, resource_records)
        }
    };
    CommandOutputPlan::result(json!({
        "frameTree": frame_tree
    }))
}

fn frame_tree_payload(
    target_id: String,
    target_loader_id: String,
    target_url: String,
    target_unreachable_url: Option<String>,
    target_security_origin: String,
    target_secure_context_type: String,
    target_mime_type: String,
    child_frame_snapshots: Vec<ChildFrameTreeSnapshot>,
) -> Value {
    let mut frame_tree = json!({
        "frame": {
            "id": target_id,
            "loaderId": target_loader_id,
            "url": target_url,
            "domainAndRegistry": "",
            "securityOrigin": target_security_origin,
            "mimeType": target_mime_type,
            "adFrameStatus": { "adFrameType": "none" },
            "secureContextType": target_secure_context_type,
            "crossOriginIsolatedContextType": "NotIsolated",
            "gatedAPIFeatures": [],
        }
    });
    if let Some(unreachable_url) = target_unreachable_url {
        frame_tree["frame"]["unreachableUrl"] = json!(unreachable_url);
    }
    let child_frames = child_frame_snapshots
        .into_iter()
        .map(|frame| {
            build_child_frame_tree_payload(
                &frame,
                &target_id,
                &target_security_origin,
                &target_secure_context_type,
            )
        })
        .collect::<Vec<_>>();
    if !child_frames.is_empty() {
        frame_tree["childFrames"] = Value::Array(child_frames);
    }
    frame_tree
}

pub(super) fn build_child_frame_tree_payload(
    frame: &moli_core::page::ChildFrameTreeSnapshot,
    parent_frame_id: &str,
    inherited_security_origin: &str,
    inherited_secure_context_type: &str,
) -> Value {
    let (security_origin, secure_context_type) = child_frame_security_identity(
        &frame.url,
        frame.security_origin_inherited,
        frame.security_origin_opaque,
        inherited_security_origin,
        inherited_secure_context_type,
    );
    let name = frame
        .name
        .as_deref()
        .filter(|name| !name.is_empty())
        .or(frame.owner_element_id.as_deref())
        .unwrap_or("");
    let frame_payload = json!({
        "id": frame.frame_id,
        "parentId": parent_frame_id,
        "loaderId": if frame.loader_id.is_empty() { LOADER_ID } else { &frame.loader_id },
        "name": name,
        "url": frame.url,
        "domainAndRegistry": "",
        "securityOrigin": security_origin.clone(),
        "mimeType": "text/html",
        "adFrameStatus": { "adFrameType": "none" },
        "secureContextType": secure_context_type.clone(),
        "crossOriginIsolatedContextType": "NotIsolated",
        "gatedAPIFeatures": [],
    });
    let mut payload = json!({ "frame": frame_payload });
    if !frame.child_frames.is_empty() {
        payload["childFrames"] = Value::Array(
            frame
                .child_frames
                .iter()
                .map(|child| {
                    build_child_frame_tree_payload(
                        child,
                        &frame.frame_id,
                        &security_origin,
                        &secure_context_type,
                    )
                })
                .collect(),
        );
    }
    payload
}

pub(crate) fn child_frame_security_identity(
    url: &str,
    security_origin_inherited: bool,
    _security_origin_opaque: bool,
    _inherited_security_origin: &str,
    inherited_secure_context_type: &str,
) -> (String, String) {
    let parsed_url = Url::parse(url).ok();
    // Blink's Page.Frame projection constructs this field from the
    // DocumentLoader URL, not from the document's live SecurityOrigin.
    let security_origin = parsed_url
        .as_ref()
        .map(|url| {
            if url.scheme() == "about" {
                "://".to_owned()
            } else {
                moli_url::origin_ascii_serialization(url)
            }
        })
        .unwrap_or_else(|| "null".to_owned());
    let inherited_secure_context =
        security_origin_inherited || parsed_url.as_ref().is_some_and(moli_url::is_about_blank);
    let secure_context_type = if inherited_secure_context {
        inherited_secure_context_type.to_owned()
    } else if parsed_url
        .as_ref()
        .is_some_and(moli_url::is_potentially_trustworthy_url)
    {
        "Secure".to_owned()
    } else {
        "InsecureScheme".to_owned()
    };
    (security_origin, secure_context_type)
}

pub(super) async fn execute_devtools_get_frame_trees_command_async(
    conn: &mut CdpConnection,
    command: DevToolsGetFrameTreesCommand,
) -> Result<AutomationResult, DevToolsError> {
    let mut frame_trees = Vec::new();
    for target_info in devtools_browsing_context_target_infos(conn) {
        let Some(target_id) = target_info.target_id.clone() else {
            continue;
        };
        let frame_tree_command = DevToolsGetFrameTreeCommand {
            context: AutomationContext {
                target_id: Some(target_id),
                ..command.context.clone()
            },
            max_depth: command.max_depth,
        };
        let AutomationResult::GetFrameTree(frame_tree_result) =
            execute_devtools_get_frame_tree_command_async(conn, frame_tree_command).await?
        else {
            return Err(DevToolsError::new(
                DevToolsErrorKind::Internal,
                "UnexpectedFrameTreeResult",
            ));
        };
        frame_trees.push(frame_tree_result);
    }
    Ok(AutomationResult::GetFrameTrees(
        DevToolsGetFrameTreesResult { frame_trees },
    ))
}

pub(super) fn devtools_browsing_context_target_infos(
    conn: &CdpConnection,
) -> Vec<DevToolsTargetInfo> {
    conn.browser_contexts()
        .flat_map(|browser_context| browser_context.devtools_target_infos())
        .filter(|info| {
            matches!(
                info.kind,
                DevToolsTargetKind::Page
                    | DevToolsTargetKind::Frame
                    | DevToolsTargetKind::ServiceWorker
            )
        })
        .collect()
}

pub(super) async fn execute_devtools_get_frame_tree_command_async(
    conn: &mut CdpConnection,
    command: DevToolsGetFrameTreeCommand,
) -> Result<AutomationResult, DevToolsError> {
    let target_info = command
        .context
        .target_id
        .as_ref()
        .and_then(|target_id| devtools_target_info_for_target_id(conn, target_id.as_str()));
    if matches!(
        target_info.as_ref().map(|info| info.kind),
        Some(DevToolsTargetKind::ServiceWorker)
    ) {
        return devtools_service_worker_frame_tree_result(
            target_info.expect("service worker target info was checked"),
            command.max_depth,
        )
        .map(AutomationResult::GetFrameTree);
    }
    let owner = page_command_owner(conn, &command.context)?;
    let max_depth = command.max_depth;
    devtools_frame_tree_for_current_owner_async(conn, command, &owner)
        .await
        .map(|frame_tree| {
            AutomationResult::GetFrameTree(DevToolsGetFrameTreeResult {
                frame_tree,
                target_info,
                max_depth,
            })
        })
}

pub(super) fn devtools_service_worker_frame_tree_result(
    target_info: DevToolsTargetInfo,
    max_depth: Option<u32>,
) -> Result<DevToolsGetFrameTreeResult, DevToolsError> {
    let Some(target_id) = target_info.target_id.as_ref() else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "MissingServiceWorkerTargetId",
        ));
    };
    Ok(DevToolsGetFrameTreeResult {
        frame_tree: json!({
            "frame": {
                "id": target_id.as_str(),
                "url": target_info.url.as_str(),
            }
        }),
        target_info: Some(target_info),
        max_depth,
    })
}

pub(super) async fn devtools_frame_tree_for_current_owner_async(
    conn: &mut CdpConnection,
    command: DevToolsGetFrameTreeCommand,
    owner: &CommandOwnerScope,
) -> Result<Value, DevToolsError> {
    let command_session_id = command.context.session_id.as_ref().map(|id| id.as_str());
    if command_session_id.is_none() && conn.browser_context.is_none() {
        return Err(devtools_frame_tree_error("BrowserContextNotLoaded"));
    }
    let (target_id, target_url, target_security_origin, target_secure_context_type) = conn
        .target_session_owner_frame_tree_identity_for_owner(owner)
        .ok_or_else(|| devtools_frame_tree_error("TargetNotLoaded"))?;
    let target_unreachable_url = network_error_page_unreachable_url(conn, owner, &target_url);
    let target_loader_id = frame_tree_loader_id_for_current_owner(conn, owner);
    if conn.ensure_document_accessible_for_owner(owner).is_err() {
        return Ok(frame_tree_payload(
            target_id,
            target_loader_id,
            target_url,
            target_unreachable_url,
            target_security_origin,
            target_secure_context_type,
            default_document_mime_type(),
            Vec::new(),
        ));
    }
    let Some(page) = conn
        .runtime_session_owner_slot_mut_for_owner(owner)
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return Ok(frame_tree_payload(
            target_id,
            target_loader_id,
            target_url,
            target_unreachable_url,
            target_security_origin,
            target_secure_context_type,
            default_document_mime_type(),
            Vec::new(),
        ));
    };
    let target_mime_type = main_document_mime_type(page);
    let pending = page.start_child_frame_tree_snapshot().map_err(|error| {
        devtools_frame_tree_error(format!("Failed to snapshot child frame tree: {error}"))
    })?;
    let completed = pending.wait().await.map_err(|error| {
        devtools_frame_tree_error(format!("Failed to snapshot child frame tree: {error}"))
    })?;
    if conn.ensure_document_accessible_for_owner(owner).is_err() {
        return Ok(frame_tree_payload(
            target_id,
            target_loader_id,
            target_url,
            target_unreachable_url,
            target_security_origin,
            target_secure_context_type,
            target_mime_type,
            Vec::new(),
        ));
    }
    let Some(page) = conn
        .runtime_session_owner_slot_mut_for_owner(owner)
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return Ok(frame_tree_payload(
            target_id,
            target_loader_id,
            target_url,
            target_unreachable_url,
            target_security_origin,
            target_secure_context_type,
            target_mime_type,
            Vec::new(),
        ));
    };
    let child_frames = page
        .finish_child_frame_tree_snapshot(completed)
        .map_err(|error| {
            devtools_frame_tree_error(format!("Failed to snapshot child frame tree: {error}"))
        })?;
    Ok(frame_tree_payload(
        target_id,
        target_loader_id,
        target_url,
        target_unreachable_url,
        target_security_origin,
        target_secure_context_type,
        target_mime_type,
        child_frames,
    ))
}

pub(super) fn default_document_mime_type() -> String {
    "text/html".to_owned()
}

pub(super) fn network_error_page_unreachable_url(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
    document_url: &str,
) -> Option<String> {
    (document_url == NETWORK_ERROR_PAGE_URL)
        .then(|| conn.runtime_session_owner_target_url_for_owner(owner))
        .flatten()
}

pub(super) fn main_document_mime_type(page: &Page) -> String {
    moli_web_mime::effective_response_mime_essence(page.headers(), None)
        .unwrap_or_else(default_document_mime_type)
}

pub(super) fn devtools_frame_tree_error(message: impl Into<String>) -> DevToolsError {
    DevToolsError::new(DevToolsErrorKind::Internal, message)
}

/// Returns the current committed DocumentLoader identity for CDP serialization.
///
/// Blink's `BuildObjectForFrame()` permits the rare state where a `LocalFrame`
/// has no `DocumentLoader`; `IdentifiersFactory::LoaderId(nullptr)` serializes
/// that absence as an empty string. Reusing a well-known loader ID here would
/// instead claim that the frame belongs to an unrelated document/navigation.
pub(super) fn frame_tree_loader_id_for_current_owner(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
) -> String {
    conn.target_session_owner_frame_tree_loader_id_for_owner(owner)
        .unwrap_or_default()
}

pub(super) fn devtools_target_info_for_target_id(
    conn: &CdpConnection,
    target_id: &str,
) -> Option<DevToolsTargetInfo> {
    conn.browser_contexts()
        .find_map(|browser_context| browser_context.devtools_target_info(target_id))
}

pub(super) fn try_start_page_get_frame_tree_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    let command = build_cdp_get_frame_tree_command(conn, cmd);
    start_devtools_get_frame_tree_command(
        conn,
        cmd.id,
        command,
        FrameTreeCommandOutputKind::FrameTree,
    )
}

pub(super) fn try_start_page_get_resource_tree_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    let command = build_cdp_get_frame_tree_command(conn, cmd);
    start_devtools_get_frame_tree_command(
        conn,
        cmd.id,
        command,
        FrameTreeCommandOutputKind::ResourceTree,
    )
}

pub(super) fn build_cdp_get_frame_tree_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> DevToolsGetFrameTreeCommand {
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    DevToolsGetFrameTreeCommand {
        context: cmd.automation_context(target_id.as_deref(), browser_context_id.as_deref()),
        max_depth: None,
    }
}

pub(super) fn start_devtools_get_frame_tree_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command: DevToolsGetFrameTreeCommand,
    output_kind: FrameTreeCommandOutputKind,
) -> PageCommandTaskStep {
    let command_session_id = command.context.session_id.as_ref().map(|id| id.as_str());
    if command_session_id.is_none() && conn.browser_context.is_none() {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -31998,
            "BrowserContextNotLoaded",
        ));
    };
    let owner = CommandOwnerScope::capture(conn, command_session_id);
    let (target_id, target_url, target_security_origin, target_secure_context_type) =
        match conn.target_session_owner_frame_tree_identity_for_owner(&owner) {
            Some(identity) => identity,
            None => {
                return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                    -31998,
                    "TargetNotLoaded",
                ));
            }
        };
    let target_unreachable_url = network_error_page_unreachable_url(conn, &owner, &target_url);
    let target_url = if pending_initial_document_url(conn, &owner).is_some() {
        ":".to_owned()
    } else {
        target_url
    };
    let target_loader_id = frame_tree_loader_id_for_current_owner(conn, &owner);
    if conn.ensure_document_accessible_for_owner(&owner).is_err() {
        return PageCommandTaskStep::Complete(get_frame_tree_command_output_plan(
            output_kind,
            target_id,
            target_loader_id,
            target_url,
            target_unreachable_url,
            target_security_origin,
            target_secure_context_type,
            default_document_mime_type(),
            Vec::new(),
            &[],
        ));
    }
    let Some(page) = conn
        .runtime_session_owner_slot_mut_for_owner(&owner)
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return PageCommandTaskStep::Complete(get_frame_tree_command_output_plan(
            output_kind,
            target_id,
            target_loader_id,
            target_url,
            target_unreachable_url,
            target_security_origin,
            target_secure_context_type,
            default_document_mime_type(),
            Vec::new(),
            &[],
        ));
    };
    let target_mime_type = main_document_mime_type(page);
    match page.start_child_frame_tree_snapshot() {
        Ok(pending) => PageCommandTaskStep::Pending(PendingPageCommandDispatch {
            command_id,
            owner_scope: CommandOwnerScope::capture(conn, command_session_id),
            kind: Box::new(PendingPageCommandKind::GetFrameTree {
                output_kind,
                target_id,
                target_loader_id,
                target_url,
                target_unreachable_url,
                target_security_origin,
                target_secure_context_type,
                target_mime_type,
                pending,
            }),
        }),
        Err(error) => PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            format!("Failed to snapshot child frame tree: {error}"),
        )),
    }
}

use std::collections::HashMap;

use moli_core::page::{
    SubresourceNetworkOutcome, SubresourceNetworkRecord, SubresourceResourceType,
};
use moli_web_mime::effective_response_mime_essence;
use serde_json::{Value, json};
use url::Url;

#[derive(Debug)]
struct FrameResourceSnapshot {
    frame_id: Option<String>,
    url: String,
    payload: Value,
}

pub(super) fn attach_frame_resources(
    mut frame_tree: Value,
    records: &[SubresourceNetworkRecord],
) -> Value {
    let resources = frame_resource_snapshots(records);
    attach_resources_to_frame(&mut frame_tree, &resources, true);
    frame_tree
}

fn frame_resource_snapshots(records: &[SubresourceNetworkRecord]) -> Vec<FrameResourceSnapshot> {
    let mut snapshots = Vec::new();
    let mut index_by_resource = HashMap::new();

    for record in records {
        let Some(snapshot) = frame_resource_snapshot(record) else {
            continue;
        };
        let key = (snapshot.frame_id.clone(), snapshot.url.clone());
        if let Some(index) = index_by_resource.get(&key).copied() {
            snapshots[index] = snapshot;
        } else {
            index_by_resource.insert(key, snapshots.len());
            snapshots.push(snapshot);
        }
    }

    snapshots
}

fn frame_resource_snapshot(record: &SubresourceNetworkRecord) -> Option<FrameResourceSnapshot> {
    if !resource_type_appears_in_frame_tree(record.resource_type()) {
        return None;
    }

    let (url, mime_type, content_size, failed) = match record.outcome() {
        SubresourceNetworkOutcome::Success {
            final_url,
            response_headers,
            response_body,
            ..
        } => (
            url_without_fragment(final_url),
            effective_response_mime_essence(response_headers, None).unwrap_or_default(),
            response_body.len(),
            false,
        ),
        SubresourceNetworkOutcome::Failure { .. } => {
            (url_without_fragment(record.url()), String::new(), 0, true)
        }
    };

    let mut payload = json!({
        "url": url,
        "type": record.resource_type().as_cdp_type(),
        "mimeType": mime_type,
        "contentSize": content_size,
    });
    if failed {
        payload["failed"] = json!(true);
    }

    Some(FrameResourceSnapshot {
        frame_id: record.frame_id().map(str::to_owned),
        url,
        payload,
    })
}

fn resource_type_appears_in_frame_tree(resource_type: SubresourceResourceType) -> bool {
    !matches!(
        resource_type,
        SubresourceResourceType::Fetch
            | SubresourceResourceType::EventSource
            | SubresourceResourceType::Xhr
            | SubresourceResourceType::Ping
            | SubresourceResourceType::CspReport
            | SubresourceResourceType::WebSocket
    )
}

fn url_without_fragment(url: &Url) -> String {
    let mut url = url.clone();
    url.set_fragment(None);
    url.into()
}

fn attach_resources_to_frame(
    frame_tree: &mut Value,
    resources: &[FrameResourceSnapshot],
    is_root: bool,
) {
    let frame_id = frame_tree["frame"]["id"].as_str().unwrap_or_default();
    frame_tree["resources"] = Value::Array(
        resources
            .iter()
            .filter(|resource| {
                resource.frame_id.as_deref() == Some(frame_id)
                    || (is_root && resource.frame_id.is_none())
            })
            .map(|resource| resource.payload.clone())
            .collect(),
    );

    let Some(child_frames) = frame_tree
        .get_mut("childFrames")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for child_frame in child_frames {
        attach_resources_to_frame(child_frame, resources, false);
    }
}

/// Blink initializes a DocumentLoader with an empty URL, while the initial
/// DOM document exposes about:blank. InspectorPageAgent serializes that empty
/// loader URL as ":". Moli materializes both using the initial document URL,
/// so retain the lifecycle distinction until the target's first navigation
/// replaces it. An ordinary ready about:blank target needs no replacement.
/// This is independent of debugger suspension and response delivery mode.
fn pending_initial_document_url(conn: &CdpConnection, owner: &CommandOwnerScope) -> Option<String> {
    conn.runtime_session_owner_initial_empty_document_has_replacement_url_for_owner(owner)
        .then(|| conn.runtime_session_owner_record_initial_empty_document_url_for_owner(owner))
        .flatten()
}

/// Resolve both children and resource records before publishing the terminal.
/// Browser identity is fixed at admission; URL, MIME and resource content come
/// from the renderer's immutable state for the completed command turn.
pub(super) fn prepare_native_tree(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    resources: bool,
) -> Result<moli_core::RendererNativeOperation, CommandOutputPlan> {
    use moli_core::{
        RendererNativeOperation as Operation, RendererNativeProtocolResponse as Response,
        RendererPageCommand as Command, RendererPageReply as Reply,
    };
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let (target_id, target_url, security_origin, secure_context_type) = conn
        .target_session_owner_frame_tree_identity_for_owner(&owner)
        .ok_or_else(|| CommandOutputPlan::error(-31998, "TargetNotLoaded"))?;
    let unreachable_url = network_error_page_unreachable_url(conn, &owner, &target_url);
    let loader_id = frame_tree_loader_id_for_current_owner(conn, &owner);
    let initial_document_url = pending_initial_document_url(conn, &owner);
    Ok(Operation::with_page_state(
        Command::ChildFrameTreeSnapshot,
        move |reply, state| match reply {
            Ok(Reply::ChildFrameTreeSnapshots(children)) => {
                // A preceding renderer command may already have changed the
                // document URL before its Browser adapter applies the state.
                // Only project the initial loader while that document is current.
                let document_url =
                    if initial_document_url.as_deref() == Some(state.final_url.as_str()) {
                        ":".to_owned()
                    } else {
                        state.final_url.to_string()
                    };
                let tree = frame_tree_payload(
                    target_id,
                    loader_id,
                    document_url,
                    unreachable_url,
                    security_origin,
                    secure_context_type,
                    moli_web_mime::effective_response_mime_essence(&state.headers, None)
                        .unwrap_or_else(default_document_mime_type),
                    children,
                );
                let tree = if resources {
                    attach_frame_resources(
                        tree,
                        state.script_execution.subresource_network_records(),
                    )
                } else {
                    tree
                };
                Response::success(json!({"frameTree": tree}))
            }
            Err(error) => Response::error(
                -32000,
                format!("Failed to snapshot child frame tree: {error}"),
            ),
            _ => unreachable!("Page frame tree reply"),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn successful_record(
        frame_id: Option<&str>,
        requested_url: &str,
        final_url: &str,
        resource_type: SubresourceResourceType,
        content_type: &str,
        body: &str,
    ) -> SubresourceNetworkRecord {
        SubresourceNetworkRecord::success(
            frame_id.map(str::to_owned),
            Url::parse("https://example.test/document").unwrap(),
            Url::parse(requested_url).unwrap(),
            "GET".to_owned(),
            Vec::new().into(),
            None,
            resource_type,
            None,
            Vec::new(),
            Url::parse(final_url).unwrap(),
            200,
            vec![("Content-Type".to_owned(), content_type.as_bytes().to_vec())],
            body.to_owned(),
            Vec::new(),
        )
    }

    #[test]
    fn resource_tree_uses_observed_final_response_metadata() {
        let records = vec![successful_record(
            None,
            "https://example.test/original.js",
            "https://cdn.example.test/app.js#cache-key",
            SubresourceResourceType::Script,
            "application/javascript; charset=utf-8",
            "window.loaded = true;",
        )];

        let tree = attach_frame_resources(
            json!({"frame": {"id": "ROOT"}, "childFrames": []}),
            &records,
        );

        assert_eq!(
            tree["resources"],
            json!([{
                "url": "https://cdn.example.test/app.js",
                "type": "Script",
                "mimeType": "application/javascript",
                "contentSize": 21,
            }])
        );
    }

    #[test]
    fn resource_tree_attributes_children_and_omits_raw_resources() {
        let records = vec![
            successful_record(
                Some("CHILD"),
                "https://example.test/child.css",
                "https://example.test/child.css",
                SubresourceResourceType::Stylesheet,
                "text/css",
                "p{}",
            ),
            successful_record(
                None,
                "https://example.test/data",
                "https://example.test/data",
                SubresourceResourceType::Fetch,
                "application/json",
                "{}",
            ),
            successful_record(
                Some("DETACHED"),
                "https://example.test/stale.js",
                "https://example.test/stale.js",
                SubresourceResourceType::Script,
                "text/javascript",
                "0",
            ),
        ];

        let tree = attach_frame_resources(
            json!({
                "frame": {"id": "ROOT"},
                "childFrames": [{"frame": {"id": "CHILD"}}],
            }),
            &records,
        );

        assert_eq!(tree["resources"], json!([]));
        assert_eq!(
            tree["childFrames"][0]["resources"],
            json!([{
                "url": "https://example.test/child.css",
                "type": "Stylesheet",
                "mimeType": "text/css",
                "contentSize": 3,
            }])
        );
    }

    #[test]
    fn resource_tree_replaces_duplicate_cache_entries_and_marks_failures() {
        let failed = SubresourceNetworkRecord::failure(
            None,
            Url::parse("https://example.test/document").unwrap(),
            Url::parse("https://example.test/app.js#fragment").unwrap(),
            "GET".to_owned(),
            Vec::new().into(),
            None,
            SubresourceResourceType::Script,
            "connection reset".to_owned(),
        );
        let successful = successful_record(
            None,
            "https://example.test/app.js",
            "https://example.test/app.js",
            SubresourceResourceType::Script,
            "text/javascript",
            "ok",
        );

        let failed_tree = attach_frame_resources(
            json!({"frame": {"id": "ROOT"}}),
            std::slice::from_ref(&failed),
        );
        assert_eq!(failed_tree["resources"][0]["failed"], json!(true));
        assert_eq!(failed_tree["resources"][0]["contentSize"], json!(0));

        let recovered_tree =
            attach_frame_resources(json!({"frame": {"id": "ROOT"}}), &[failed, successful]);
        assert_eq!(recovered_tree["resources"].as_array().unwrap().len(), 1);
        assert_eq!(
            recovered_tree["resources"][0],
            json!({
                "url": "https://example.test/app.js",
                "type": "Script",
                "mimeType": "text/javascript",
                "contentSize": 2,
            })
        );
    }
}

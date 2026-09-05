use moli_renderer_v8::RendererDomInspection;

use serde_json::json;

use super::{
    dom_agent_includes_whitespace_for_owner, frontend_binding, node_snapshot_to_cdp,
    resolve::{
        PendingDomCommandStartError, attributes_result_from_renderer_resolution,
        devtools_dom_geometry_result_from_renderer, property_result_from_renderer_resolution,
        query_selector_result_from_renderer_resolution, renderer_backend_node_id_for_reference,
        text_result_from_renderer_resolution,
    },
};
use crate::conn::{CdpConnection, CommandOwnerScope};
use crate::devtools_runtime::{
    DevToolsCommand, DevToolsCommandResult, DevToolsDescribeNodeCommand,
    DevToolsDescribeNodeResult, DevToolsDomGeometryCommand, DevToolsDomGeometryResult,
    DevToolsDomNodeReference, DevToolsGetAttributesResult, DevToolsGetOuterHtmlCommand,
    DevToolsGetOuterHtmlResult, DevToolsGetPropertyResult, DevToolsGetTextResult,
    DevToolsQuerySelectorResult, DevToolsResolveNodeCommand, DevToolsResolveNodeResult,
    DevToolsScrollIntoViewIfNeededCommand,
};
use moli_core::page::{
    DocumentNodeRuntimeObjectResolution, PendingPageCommand, RendererDocumentNodeGeometry,
    RendererDocumentNodeReference,
};

use super::dom_inspection_for_owner;

pub(super) async fn execute_devtools_dom_command(
    conn: &mut CdpConnection,
    frame_id: &str,
    owner: &CommandOwnerScope,
    command: DevToolsCommand,
) -> Result<DevToolsCommandResult, PendingDomCommandStartError> {
    match command {
        DevToolsCommand::QuerySelector(command) => {
            let result = query_selector_command(
                conn,
                owner,
                frame_id,
                command.root,
                &command.selector,
                command.multiple,
            )
            .await?;
            Ok(DevToolsCommandResult::QuerySelector(result))
        }
        DevToolsCommand::GetAttributes(command) => {
            let result = attributes_command(conn, owner, command.reference).await?;
            Ok(DevToolsCommandResult::GetAttributes(result))
        }
        DevToolsCommand::GetText(command) => {
            let result = text_command(conn, owner, command.reference).await?;
            Ok(DevToolsCommandResult::GetText(result))
        }
        DevToolsCommand::GetProperty(command) => {
            let result = property_command(conn, owner, command.reference, &command.name).await?;
            Ok(DevToolsCommandResult::GetProperty(result))
        }
        DevToolsCommand::GetOuterHtml(command) => {
            let DevToolsGetOuterHtmlCommand {
                context: _,
                reference,
                include_shadow_dom,
            } = command;
            let outer_html =
                outer_html_command(conn, owner, frame_id, reference, include_shadow_dom).await?;
            Ok(DevToolsCommandResult::GetOuterHtml(
                DevToolsGetOuterHtmlResult { outer_html },
            ))
        }
        DevToolsCommand::DescribeNode(command) => {
            let result = describe_node_command(conn, owner, frame_id, command).await?;
            Ok(DevToolsCommandResult::DescribeNode(result))
        }
        DevToolsCommand::ResolveNode(command) => {
            let result = resolve_node_command(conn, owner, command).await?;
            Ok(DevToolsCommandResult::ResolveNode(result))
        }
        DevToolsCommand::DomGeometry(command) => {
            let result = dom_geometry_command(conn, owner, command).await?;
            Ok(DevToolsCommandResult::DomGeometry(result))
        }
        DevToolsCommand::ScrollIntoViewIfNeeded(command) => {
            scroll_into_view_if_needed_command(conn, owner, command).await?;
            Ok(DevToolsCommandResult::Empty)
        }
        _ => Err(PendingDomCommandStartError::no_such_target()),
    }
}

async fn query_selector_command(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    frame_id: &str,
    root: Option<DevToolsDomNodeReference>,
    selector: &str,
    multiple: bool,
) -> Result<DevToolsQuerySelectorResult, PendingDomCommandStartError> {
    let include_whitespace = dom_agent_includes_whitespace_for_owner(conn, owner);
    let root_backend_node_id = match root {
        Some(reference) => {
            let reference = resolve_frontend_node_reference(conn, owner, reference).await?;
            required_child_frame_backend_node_id(&reference)?
        }
        None => {
            child_frame_document_root_node_reference(conn, owner, frame_id)
                .await?
                .backend_node_id
        }
    };
    let inspection = dom_inspection_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = inspection
        .start_child_frame_document_query_selector_for_backend_node_id(
            include_whitespace,
            frame_id.to_owned(),
            root_backend_node_id,
            selector.to_owned(),
            multiple,
        )
        .map(PendingPageCommand::from_inspector_main_route)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    let completion = pending
        .wait()
        .await
        .map_err(PendingDomCommandStartError::renderer_error)?;
    conn.observe_renderer_inspection_completion(owner, &completion)
        .map_err(|message| PendingDomCommandStartError {
            code: -32000,
            message,
        })?;
    let resolution = completion
        .finish_document_query_selector()
        .map_err(PendingDomCommandStartError::renderer_error)?;
    query_selector_result_from_renderer_resolution(resolution, multiple)
}

async fn resolve_frontend_node_reference(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    reference: DevToolsDomNodeReference,
) -> Result<DevToolsDomNodeReference, PendingDomCommandStartError> {
    let DevToolsDomNodeReference::FrontendNodeId(frontend_node_id) = reference else {
        return Ok(reference);
    };

    let inspection = dom_inspection_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = inspection
        .start_document_frontend_node_binding(frontend_node_id)
        .map(PendingPageCommand::from_inspector_main_route)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    let completion = pending
        .wait()
        .await
        .map_err(PendingDomCommandStartError::renderer_error)?;
    conn.observe_renderer_inspection_completion(owner, &completion)
        .map_err(|message| PendingDomCommandStartError {
            code: -32000,
            message,
        })?;

    frontend_binding::finish_reference(completion).map_err(|message| PendingDomCommandStartError {
        code: -32000,
        message,
    })
}

async fn child_frame_document_root_node_reference(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    frame_id: &str,
) -> Result<RendererDocumentNodeReference, PendingDomCommandStartError> {
    let inspection = dom_inspection_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = inspection
        .start_child_frame_document_root_node_reference(frame_id)
        .map(PendingPageCommand::from_inspector_main_route)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    let completion = pending
        .wait()
        .await
        .map_err(PendingDomCommandStartError::renderer_error)?;
    conn.observe_renderer_inspection_completion(owner, &completion)
        .map_err(|message| PendingDomCommandStartError {
            code: -32000,
            message,
        })?;

    completion
        .finish_document_node_reference()
        .map_err(PendingDomCommandStartError::renderer_error)?
        .ok_or_else(PendingDomCommandStartError::node_not_found)
}

async fn attributes_command(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    reference: DevToolsDomNodeReference,
) -> Result<DevToolsGetAttributesResult, PendingDomCommandStartError> {
    let reference = resolve_frontend_node_reference(conn, owner, reference).await?;
    let inspection = dom_inspection_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = start_document_node_attributes_for_reference(&inspection, reference)?;
    let completion = pending
        .wait()
        .await
        .map_err(PendingDomCommandStartError::renderer_error)?;
    conn.observe_renderer_inspection_completion(owner, &completion)
        .map_err(|message| PendingDomCommandStartError {
            code: -32000,
            message,
        })?;
    completion
        .finish_document_node_attributes()
        .map_err(PendingDomCommandStartError::renderer_error)
        .and_then(attributes_result_from_renderer_resolution)
}

fn start_document_node_attributes_for_reference(
    inspection: &RendererDomInspection<'_>,
    reference: DevToolsDomNodeReference,
) -> Result<PendingPageCommand, PendingDomCommandStartError> {
    let backend_node_id = required_child_frame_backend_node_id(&reference)?;
    inspection
        .start_document_node_attributes_for_backend_node_id(backend_node_id)
        .map(PendingPageCommand::from_inspector_main_route)
        .map_err(PendingDomCommandStartError::renderer_error)
}

async fn text_command(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    reference: DevToolsDomNodeReference,
) -> Result<DevToolsGetTextResult, PendingDomCommandStartError> {
    let reference = resolve_frontend_node_reference(conn, owner, reference).await?;
    let inspection = dom_inspection_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = start_document_node_text_for_reference(&inspection, reference)?;
    let completion = pending
        .wait()
        .await
        .map_err(PendingDomCommandStartError::renderer_error)?;
    conn.observe_renderer_inspection_completion(owner, &completion)
        .map_err(|message| PendingDomCommandStartError {
            code: -32000,
            message,
        })?;
    completion
        .finish_document_node_text()
        .map_err(PendingDomCommandStartError::renderer_error)
        .and_then(text_result_from_renderer_resolution)
}

fn start_document_node_text_for_reference(
    inspection: &RendererDomInspection<'_>,
    reference: DevToolsDomNodeReference,
) -> Result<PendingPageCommand, PendingDomCommandStartError> {
    let backend_node_id = required_child_frame_backend_node_id(&reference)?;
    inspection
        .start_document_node_text_for_backend_node_id(backend_node_id)
        .map(PendingPageCommand::from_inspector_main_route)
        .map_err(PendingDomCommandStartError::renderer_error)
}

async fn property_command(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    reference: DevToolsDomNodeReference,
    name: &str,
) -> Result<DevToolsGetPropertyResult, PendingDomCommandStartError> {
    let reference = resolve_frontend_node_reference(conn, owner, reference).await?;
    let inspection = dom_inspection_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = start_document_node_property_for_reference(&inspection, reference, name)?;
    let completion = pending
        .wait()
        .await
        .map_err(PendingDomCommandStartError::renderer_error)?;
    conn.observe_renderer_inspection_completion(owner, &completion)
        .map_err(|message| PendingDomCommandStartError {
            code: -32000,
            message,
        })?;
    completion
        .finish_document_node_property()
        .map_err(PendingDomCommandStartError::renderer_error)
        .and_then(property_result_from_renderer_resolution)
}

fn start_document_node_property_for_reference(
    inspection: &RendererDomInspection<'_>,
    reference: DevToolsDomNodeReference,
    name: &str,
) -> Result<PendingPageCommand, PendingDomCommandStartError> {
    let backend_node_id = required_child_frame_backend_node_id(&reference)?;
    inspection
        .start_document_node_property_for_backend_node_id(backend_node_id, name)
        .map(PendingPageCommand::from_inspector_main_route)
        .map_err(PendingDomCommandStartError::renderer_error)
}

async fn outer_html_command(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    frame_id: &str,
    reference: Option<DevToolsDomNodeReference>,
    include_shadow_dom: bool,
) -> Result<String, PendingDomCommandStartError> {
    let reference = match reference {
        Some(reference) => Some(resolve_frontend_node_reference(conn, owner, reference).await?),
        None => None,
    };
    let backend_node_id = match reference {
        Some(reference) => required_child_frame_backend_node_id(&reference)?,
        None => {
            child_frame_document_root_node_reference(conn, owner, frame_id)
                .await?
                .backend_node_id
        }
    };
    let inspection = dom_inspection_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = inspection
        .start_outer_html_for_backend_node_id(backend_node_id, include_shadow_dom)
        .map(PendingPageCommand::from_inspector_main_route)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    let completion = pending
        .wait()
        .await
        .map_err(PendingDomCommandStartError::renderer_error)?;
    conn.observe_renderer_inspection_completion(owner, &completion)
        .map_err(|message| PendingDomCommandStartError {
            code: -32000,
            message,
        })?;
    completion
        .finish_outer_html_for_backend_node_id()
        .map_err(PendingDomCommandStartError::renderer_error)?
        .ok_or_else(PendingDomCommandStartError::node_not_found)
}

async fn resolve_node_command(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    command: DevToolsResolveNodeCommand,
) -> Result<DevToolsResolveNodeResult, PendingDomCommandStartError> {
    let reference = resolve_frontend_node_reference(conn, owner, command.reference).await?;
    let object_group = command.object_group;

    let remote_object = {
        let inspection = dom_inspection_for_owner(conn, owner)
            .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
        let backend_node_id = required_child_frame_backend_node_id(&reference)?;
        let pending = inspection
            .start_resolve_runtime_object_for_backend_node_id_in_inspector_session(
                backend_node_id,
                command.execution_context_id,
                object_group.as_deref(),
            )
            .map(PendingPageCommand::from_inspector_main_route)
            .map_err(PendingDomCommandStartError::renderer_error)?;
        let completion = pending
            .wait()
            .await
            .map_err(PendingDomCommandStartError::renderer_error)?;
        conn.observe_renderer_inspection_completion(owner, &completion)
            .map_err(|message| PendingDomCommandStartError {
                code: -32000,
                message,
            })?;
        match completion
            .finish_resolve_runtime_object_for_backend_node_id()
            .map_err(PendingDomCommandStartError::renderer_error)?
        {
            DocumentNodeRuntimeObjectResolution::Found(remote_object) => remote_object,
            DocumentNodeRuntimeObjectResolution::MissingContext => {
                return Err(PendingDomCommandStartError {
                    code: -32000,
                    message: "ContextNotFound".to_owned(),
                });
            }
            DocumentNodeRuntimeObjectResolution::MissingNode => {
                return Err(PendingDomCommandStartError::node_not_found());
            }
        }
    };
    let mut remote_object = remote_object.into_protocol_value();
    if let Some(remote_object) = remote_object.as_object_mut() {
        remote_object
            .entry("subtype".to_owned())
            .or_insert_with(|| json!("node"));
    }
    let result = json!({ "object": remote_object.clone() });
    if let Some(object_group) = object_group.as_deref() {
        conn.register_runtime_remote_object_ids_from_value_for_owner_with_group(
            owner,
            &result,
            object_group,
        );
    } else {
        conn.register_runtime_remote_object_ids_from_value_for_owner(owner, &result);
    }
    Ok(DevToolsResolveNodeResult {
        object: remote_object,
    })
}

async fn describe_node_command(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    frame_id: &str,
    command: DevToolsDescribeNodeCommand,
) -> Result<DevToolsDescribeNodeResult, PendingDomCommandStartError> {
    let DevToolsDescribeNodeCommand {
        context: _,
        reference,
        depth,
        pierce,
    } = command;
    let snapshot = match reference {
        Some(reference) => {
            node_snapshot_for_reference(conn, owner, reference, depth, pierce).await?
        }
        None => child_frame_root_node_snapshot(conn, owner, frame_id, depth, pierce).await?,
    };
    let Some(node) = node_snapshot_to_cdp(&snapshot, Some(snapshot.node_id), Some(frame_id)) else {
        return Err(PendingDomCommandStartError::node_not_found());
    };
    Ok(DevToolsDescribeNodeResult { node })
}

async fn node_snapshot_for_reference(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    reference: DevToolsDomNodeReference,
    depth: i32,
    pierce: bool,
) -> Result<moli_core::page::DocumentNodeSnapshot, PendingDomCommandStartError> {
    let reference = resolve_frontend_node_reference(conn, owner, reference).await?;

    let include_whitespace = dom_agent_includes_whitespace_for_owner(conn, owner);
    let inspection = dom_inspection_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let backend_node_id = required_child_frame_backend_node_id(&reference)?;
    let pending = inspection
        .start_document_node_snapshot_for_backend_node_id_in_inspector_session(
            include_whitespace,
            backend_node_id,
            depth,
            pierce,
        )
        .map(PendingPageCommand::from_inspector_main_route)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    let completion = pending
        .wait()
        .await
        .map_err(PendingDomCommandStartError::renderer_error)?;
    conn.observe_renderer_inspection_completion(owner, &completion)
        .map_err(|message| PendingDomCommandStartError {
            code: -32000,
            message,
        })?;
    completion
        .finish_document_node_snapshot_for_backend_node_id()
        .map_err(PendingDomCommandStartError::renderer_error)?
        .map(|snapshot| snapshot.snapshot)
        .ok_or_else(PendingDomCommandStartError::node_not_found)
}

async fn child_frame_root_node_snapshot(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    frame_id: &str,
    depth: i32,
    pierce: bool,
) -> Result<moli_core::page::DocumentNodeSnapshot, PendingDomCommandStartError> {
    let backend_node_id = child_frame_document_root_node_reference(conn, owner, frame_id)
        .await?
        .backend_node_id;

    let include_whitespace = dom_agent_includes_whitespace_for_owner(conn, owner);
    let inspection = dom_inspection_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = inspection
        .start_document_node_snapshot_for_backend_node_id_in_inspector_session(
            include_whitespace,
            backend_node_id,
            depth,
            pierce,
        )
        .map(PendingPageCommand::from_inspector_main_route)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    let completion = pending
        .wait()
        .await
        .map_err(PendingDomCommandStartError::renderer_error)?;
    conn.observe_renderer_inspection_completion(owner, &completion)
        .map_err(|message| PendingDomCommandStartError {
            code: -32000,
            message,
        })?;
    completion
        .finish_document_node_snapshot_for_backend_node_id()
        .map_err(PendingDomCommandStartError::renderer_error)?
        .map(|snapshot| snapshot.snapshot)
        .ok_or_else(PendingDomCommandStartError::node_not_found)
}

async fn dom_geometry_command(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    command: DevToolsDomGeometryCommand,
) -> Result<DevToolsDomGeometryResult, PendingDomCommandStartError> {
    let geometry = document_geometry_for_reference(conn, owner, command.reference).await?;
    devtools_dom_geometry_result_from_renderer(command.operation, geometry).map_err(|error| {
        PendingDomCommandStartError {
            code: -32000,
            message: error.message,
        }
    })
}

async fn scroll_into_view_if_needed_command(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    command: DevToolsScrollIntoViewIfNeededCommand,
) -> Result<(), PendingDomCommandStartError> {
    let Some(reference) = command.reference else {
        return Err(PendingDomCommandStartError::node_not_found());
    };
    // The lightweight headless renderer currently treats scrollIntoViewIfNeeded
    // as a geometry-validating no-op. Match the top-level path for child-frame
    // elements: prove the referenced node has geometry, then complete.
    match document_geometry_for_reference(conn, owner, reference).await? {
        RendererDocumentNodeGeometry::FoundElement { .. } => Ok(()),
        RendererDocumentNodeGeometry::FoundNonElement { .. }
        | RendererDocumentNodeGeometry::NoLayoutObject
        | RendererDocumentNodeGeometry::NotElement => Err(PendingDomCommandStartError {
            code: -32000,
            message: "Node is not an element".to_owned(),
        }),
    }
}

async fn document_geometry_for_reference(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    reference: DevToolsDomNodeReference,
) -> Result<RendererDocumentNodeGeometry, PendingDomCommandStartError> {
    let reference = resolve_frontend_node_reference(conn, owner, reference).await?;
    let inspection = dom_inspection_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let backend_node_id = required_child_frame_backend_node_id(&reference)?;
    let pending = inspection
        .start_document_geometry_for_backend_node_id(backend_node_id)
        .map(PendingPageCommand::from_inspector_main_route)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    let completion = pending
        .wait()
        .await
        .map_err(PendingDomCommandStartError::renderer_error)?;
    conn.observe_renderer_inspection_completion(owner, &completion)
        .map_err(|message| PendingDomCommandStartError {
            code: -32000,
            message,
        })?;
    match completion
        .finish_document_geometry_for_backend_node_id()
        .map_err(PendingDomCommandStartError::renderer_error)?
    {
        Some(resolution) => Ok(resolution),
        None => Err(PendingDomCommandStartError::node_not_found()),
    }
}

fn required_child_frame_backend_node_id(
    reference: &DevToolsDomNodeReference,
) -> Result<u32, PendingDomCommandStartError> {
    renderer_backend_node_id_for_reference(reference)
        .ok_or_else(PendingDomCommandStartError::node_not_found)
}

use anyhow::{Context as _, Result};
use moli_core::page::{
    CompletedPageCommand, DocumentNodeRuntimeObjectResolution, Page, PendingPageCommand,
    RendererDocumentNodeGeometry, RendererDocumentNodeReference,
    RendererDomFrontendNodeBindingResolution,
};
use serde_json::json;

use super::{dom_agent_includes_whitespace_for_owner, node_snapshot_to_cdp};
use crate::automation::{
    AutomationCommand, AutomationResult, DevToolsDescribeNodeResult, DevToolsDomNodeReference,
    DevToolsError, DevToolsErrorKind, DevToolsGetOuterHtmlResult, DevToolsResolveNodeResult,
    FrontendProtocol,
};
use crate::conn::{
    CdpConnection, CommandOwnerScope, CommandWorkflowContext, ProtocolWorkflowDriver,
    RuntimePageScope,
};

pub(super) struct PreparedChildFrameDomCommand {
    scope: RuntimePageScope,
    inspector: Option<String>,
    include_whitespace: bool,
    frame_id: String,
    command: AutomationCommand,
}

impl PreparedChildFrameDomCommand {
    pub(super) fn prepare(
        conn: &CdpConnection,
        owner: CommandOwnerScope,
        frame_id: String,
        command: AutomationCommand,
    ) -> Result<Self, DevToolsError> {
        Ok(Self {
            scope: RuntimePageScope::capture(conn, &owner).map_err(dom_error)?,
            inspector: conn.target_renderer_runtime_inspector_session_id_for_owner(&owner),
            include_whitespace: dom_agent_includes_whitespace_for_owner(conn, &owner),
            frame_id,
            command,
        })
    }

    pub(super) async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
    ) -> Result<AutomationResult, DevToolsError> {
        let mut workflow = ChildFrameDomWorkflow {
            driver,
            context,
            scope: self.scope,
            inspector: self.inspector,
            include_whitespace: self.include_whitespace,
            frame_id: self.frame_id,
        };
        workflow.execute(self.command).await
    }
}

struct ChildFrameDomWorkflow<'a> {
    driver: &'a mut ProtocolWorkflowDriver,
    context: &'a mut CommandWorkflowContext,
    scope: RuntimePageScope,
    inspector: Option<String>,
    include_whitespace: bool,
    frame_id: String,
}

impl ChildFrameDomWorkflow<'_> {
    async fn query<T: 'static>(
        &mut self,
        start: impl FnOnce(&mut Page) -> Result<PendingPageCommand> + 'static,
        finish: impl FnOnce(&mut Page, CompletedPageCommand) -> Result<T> + 'static,
    ) -> Result<T, DevToolsError> {
        let scope = self.scope.clone();
        let query = self
            .context
            .with_state(self.driver, move |state, _| {
                state.prepare_dom_page_query(&scope, start, finish)
            })
            .map_err(dom_error)?
            .map_err(dom_error)?;
        query
            .run(
                self.driver,
                self.context.context_mut().renderer_output_predecessor_mut(),
            )
            .await
            .map_err(dom_error)
    }

    async fn document_root(&mut self) -> Result<RendererDocumentNodeReference, DevToolsError> {
        let frame = self.frame_id.clone();
        let inspector = self.inspector.clone();
        self.query(
            move |page| page.start_child_frame_document_root_node_reference(&frame, inspector),
            Page::finish_document_node_reference,
        )
        .await?
        .ok_or_else(node_not_found)
    }

    async fn geometry(
        &mut self,
        reference: DevToolsDomNodeReference,
        protocol: FrontendProtocol,
    ) -> Result<RendererDocumentNodeGeometry, DevToolsError> {
        let reference =
            reference.into_renderer_reference_for_protocol(self.inspector.clone(), protocol);
        self.query(
            move |page| page.start_document_geometry_for_node(reference),
            Page::finish_document_geometry_for_node,
        )
        .await?
        .ok_or_else(node_not_found)
    }

    async fn execute(
        &mut self,
        command: AutomationCommand,
    ) -> Result<AutomationResult, DevToolsError> {
        match command {
            command @ (AutomationCommand::GetAttributes(_)
            | AutomationCommand::GetText(_)
            | AutomationCommand::GetProperty(_)) => {
                let scope = self.scope.clone();
                let query = self
                    .context
                    .with_state(self.driver, move |state, _| {
                        state.prepare_scoped_dom_query(&scope, command)
                    })
                    .map_err(dom_error)??;
                query
                    .run(
                        self.driver,
                        self.context.context_mut().renderer_output_predecessor_mut(),
                    )
                    .await
            }
            AutomationCommand::QuerySelector(command) => {
                let root = match command.root {
                    Some(reference) => reference.into_renderer_reference_for_protocol(
                        self.inspector.clone(),
                        command.context.protocol,
                    ),
                    None => moli_core::page::RendererDomNodeReference::BackendNodeId(
                        self.document_root().await?.backend_node_id,
                    ),
                };
                let inspector = self.inspector.clone();
                let whitespace = self.include_whitespace;
                let frame = self.frame_id.clone();
                let multiple = command.multiple;
                let resolution = self
                    .query(
                        move |page| {
                            page.start_child_frame_document_query_selector_for_node(
                                inspector,
                                whitespace,
                                frame,
                                root,
                                command.selector,
                                multiple,
                            )
                        },
                        Page::finish_document_query_selector,
                    )
                    .await?;
                super::resolve::query_selector_result_from_renderer_resolution(resolution, multiple)
                    .map(AutomationResult::QuerySelector)
                    .map_err(DevToolsError::from)
            }
            AutomationCommand::GetOuterHtml(command) => {
                let reference = match command.reference {
                    Some(reference) => reference,
                    None => DevToolsDomNodeReference::BackendNodeId(
                        self.document_root().await?.backend_node_id,
                    ),
                }
                .into_renderer_reference_for_protocol(
                    self.inspector.clone(),
                    command.context.protocol,
                );
                let outer_html = self
                    .query(
                        move |page| {
                            page.start_outer_html_for_node(reference, command.include_shadow_dom)
                        },
                        Page::finish_outer_html_for_node,
                    )
                    .await?
                    .ok_or_else(node_not_found)?;
                Ok(AutomationResult::GetOuterHtml(DevToolsGetOuterHtmlResult {
                    outer_html,
                }))
            }
            AutomationCommand::DescribeNode(command) => {
                let reference = match command.reference {
                    Some(reference) => reference,
                    None => DevToolsDomNodeReference::BackendNodeId(
                        self.document_root().await?.backend_node_id,
                    ),
                }
                .into_renderer_reference_for_protocol(
                    self.inspector.clone(),
                    command.context.protocol,
                );
                let inspector = self.inspector.clone();
                let whitespace = self.include_whitespace;
                let snapshot = self
                    .query(
                        move |page| {
                            page.start_document_node_snapshot_for_node_in_inspector_session(
                                inspector,
                                whitespace,
                                reference,
                                command.depth,
                                command.pierce,
                            )
                        },
                        Page::finish_document_node_snapshot_for_backend_node_id,
                    )
                    .await?
                    .ok_or_else(node_not_found)?
                    .snapshot;
                let node =
                    node_snapshot_to_cdp(&snapshot, Some(snapshot.node_id), Some(&self.frame_id))
                        .ok_or_else(node_not_found)?;
                Ok(AutomationResult::DescribeNode(DevToolsDescribeNodeResult {
                    node,
                }))
            }
            AutomationCommand::ResolveNode(command) => {
                let reference = match command.reference {
                    DevToolsDomNodeReference::FrontendNodeId(id) => {
                        let inspector = self.inspector.clone();
                        let protocol = command.context.protocol;
                        self.query(
                            move |page| {
                                if protocol == FrontendProtocol::Cdp {
                                    page.start_document_frontend_node_binding(inspector, id)
                                } else {
                                    page.start_document_internal_frontend_node_binding(
                                        inspector, id,
                                    )
                                }
                            },
                            |page, completed| {
                                let binding = page
                                    .finish_document_frontend_node_binding(completed)
                                    .context("Could not resolve frontend node binding")?;
                                match binding {
                                    RendererDomFrontendNodeBindingResolution::BackendNodeId(id) => {
                                        Ok(DevToolsDomNodeReference::BackendNodeId(id))
                                    }
                                    RendererDomFrontendNodeBindingResolution::NotFound => {
                                        Err(ChildFrameDomError::NodeNotFound.into())
                                    }
                                }
                            },
                        )
                        .await?
                    }
                    reference => reference,
                };
                let backend = super::resolve::renderer_backend_node_id_for_reference(&reference)
                    .ok_or_else(node_not_found)?;
                let inspector = self.inspector.clone();
                let object_group = command.object_group;
                let native_group = object_group.clone();
                let object = self.query(move |page| {
                    page.start_resolve_runtime_object_for_backend_node_id_in_inspector_session(
                        inspector, backend, command.execution_context_id, native_group.as_deref(),
                    )
                }, Page::finish_resolve_runtime_object_for_backend_node_id).await?;
                let mut object = match object {
                    DocumentNodeRuntimeObjectResolution::Found(object) => {
                        object.into_protocol_value()
                    }
                    DocumentNodeRuntimeObjectResolution::MissingContext => {
                        return Err(dom_error(ChildFrameDomError::ContextNotFound));
                    }
                    DocumentNodeRuntimeObjectResolution::MissingNode => {
                        return Err(node_not_found());
                    }
                };
                if let Some(object) = object.as_object_mut() {
                    object
                        .entry("subtype".to_owned())
                        .or_insert_with(|| json!("node"));
                }
                let scope = self.scope.clone();
                let registered = json!({"object": object.clone()});
                self.context
                    .with_state(self.driver, move |state, _| {
                        state.register_scoped_dom_object(&scope, registered, object_group)
                    })
                    .map_err(dom_error)?
                    .map_err(dom_error)?;
                Ok(AutomationResult::ResolveNode(DevToolsResolveNodeResult {
                    object,
                }))
            }
            AutomationCommand::DomGeometry(command) => {
                let geometry = self
                    .geometry(command.reference, command.context.protocol)
                    .await?;
                super::resolve::devtools_dom_geometry_result_from_renderer(
                    command.operation,
                    geometry,
                )
                .map(AutomationResult::DomGeometry)
            }
            AutomationCommand::ScrollIntoViewIfNeeded(command) => {
                let geometry = self
                    .geometry(
                        command.reference.ok_or_else(node_not_found)?,
                        command.context.protocol,
                    )
                    .await?;
                match geometry {
                    RendererDocumentNodeGeometry::FoundElement { .. } => {
                        Ok(AutomationResult::Empty)
                    }
                    _ => Err(dom_error(ChildFrameDomError::NotElement)),
                }
            }
            _ => Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                ChildFrameDomError::NoSuchTarget,
            )),
        }
    }
}

fn dom_error(error: impl Into<anyhow::Error>) -> DevToolsError {
    let error = error.into();
    if matches!(error.downcast_ref(), Some(ChildFrameDomError::NodeNotFound)) {
        return node_not_found();
    }
    if error.is::<crate::conn::NoDocumentLoaded>() {
        return DevToolsError::new(DevToolsErrorKind::Internal, crate::conn::NoDocumentLoaded);
    }
    DevToolsError::new(DevToolsErrorKind::Internal, error)
}

fn node_not_found() -> DevToolsError {
    DevToolsError::new(
        DevToolsErrorKind::NoSuchNode,
        ChildFrameDomError::NodeNotFound,
    )
}

#[derive(Debug, thiserror::Error)]
enum ChildFrameDomError {
    #[error("Could not find node with given id")]
    NodeNotFound,
    #[error("ContextNotFound")]
    ContextNotFound,
    #[error("Node is not an element")]
    NotElement,
    #[error("NoSuchTarget")]
    NoSuchTarget,
}

use super::super::*;
use crate::runtime::page_surface::{RendererDomEdit, RendererDomEditOutcome};
use crate::script_vm::{DomInspectorEdit, DomInspectorEditOutcome};
use moli_page_types::RendererDomDebuggerDomBreakpointType;

impl PageVm {
    pub(crate) fn remove_document_node_id(&mut self, node_id: crate::dom::NodeId) -> Result<bool> {
        let payload = self.evaluate_expression_for_internal_node_reference(
            node_id,
            false,
            |token| {
                format!(
                r#"(() => {{
                    const node = __moliHostResolveInternalNodeReference({token});
                    if (!node || !node.parentNode || typeof node.parentNode.removeChild !== "function") {{
                        return false;
                    }}
                    node.parentNode.removeChild(node);
                    return true;
                }})()"#,
            )
            },
        )?;
        Ok(payload
            .get("value")
            .and_then(Value::as_bool)
            .unwrap_or(false))
    }

    pub(crate) fn remove_document_node(
        &mut self,
        reference: RendererDomNodeReference,
    ) -> Result<Option<bool>> {
        let Some(backend_node_id) = self.backend_node_id_for_document_node_reference(reference)
        else {
            return Ok(None);
        };
        let Some(handle) = self.live_handle_for_backend_node_id(backend_node_id) else {
            return Ok(Some(false));
        };
        self.remove_document_node_id(handle).map(Some)
    }

    pub(crate) fn mutate_document_node_attribute(
        &mut self,
        reference: RendererDomNodeReference,
        mutation: RendererDomAttributeMutation,
    ) -> Result<RendererDomAttributeMutationOutcome> {
        let Some(handle) = self.live_handle_for_document_node_reference(reference) else {
            return Ok(RendererDomAttributeMutationOutcome::NodeNotFound);
        };
        self.vm_mut()
            .mutate_document_node_attribute(handle, mutation)
    }

    fn live_handle_for_dom_frontend_node_id(
        &mut self,
        inspector_session_id: Option<&str>,
        frontend_node_id: u32,
    ) -> Option<DomHandle> {
        let crate::RendererDomFrontendNodeBindingResolution::BackendNodeId(backend_node_id) =
            self.published_document_frontend_node_binding(inspector_session_id, frontend_node_id)
        else {
            return None;
        };
        self.live_handle_for_backend_node_id(backend_node_id)
    }

    pub(crate) fn configure_dom_debugger_dom_breakpoint(
        &mut self,
        inspector_session_id: Option<&str>,
        frontend_node_id: u32,
        breakpoint_type: &str,
        enabled: bool,
    ) -> RendererDomDebuggerDomBreakpointResolution {
        let Some(handle) =
            self.live_handle_for_dom_frontend_node_id(inspector_session_id, frontend_node_id)
        else {
            return RendererDomDebuggerDomBreakpointResolution::NodeNotFound;
        };
        let Some(breakpoint_type) =
            RendererDomDebuggerDomBreakpointType::from_cdp_name(breakpoint_type)
        else {
            return RendererDomDebuggerDomBreakpointResolution::UnknownType(
                breakpoint_type.to_owned(),
            );
        };
        let Some(document_id) = self.vm().document_id_for_live_node_handle(handle) else {
            return RendererDomDebuggerDomBreakpointResolution::NodeNotFound;
        };
        self.vm_mut().configure_dom_debugger_dom_breakpoint(
            inspector_session_id,
            document_id,
            handle,
            breakpoint_type,
            enabled,
        );
        RendererDomDebuggerDomBreakpointResolution::Configured
    }

    pub(crate) fn edit_document_node(
        &mut self,
        inspector_session_id: Option<&str>,
        edit: RendererDomEdit,
    ) -> Result<RendererDomEditOutcome> {
        let edit = match edit {
            RendererDomEdit::MoveTo {
                node_id,
                target_node_id,
                insert_before_node_id,
            } => {
                let Some(node) =
                    self.live_handle_for_dom_frontend_node_id(inspector_session_id, node_id)
                else {
                    return Ok(RendererDomEditOutcome::NodeNotFound);
                };
                let Some(target) =
                    self.live_handle_for_dom_frontend_node_id(inspector_session_id, target_node_id)
                else {
                    return Ok(RendererDomEditOutcome::NodeNotFound);
                };
                let insert_before = match insert_before_node_id {
                    Some(node_id) => {
                        let Some(handle) = self
                            .live_handle_for_dom_frontend_node_id(inspector_session_id, node_id)
                        else {
                            return Ok(RendererDomEditOutcome::NodeNotFound);
                        };
                        Some(handle)
                    }
                    None => None,
                };
                DomInspectorEdit::MoveTo {
                    node,
                    target,
                    insert_before,
                }
            }
            RendererDomEdit::SetAttributesAsText {
                node_id,
                text,
                name,
            } => {
                let Some(node) =
                    self.live_handle_for_dom_frontend_node_id(inspector_session_id, node_id)
                else {
                    return Ok(RendererDomEditOutcome::NodeNotFound);
                };
                DomInspectorEdit::SetAttributesAsText { node, text, name }
            }
            RendererDomEdit::SetNodeName { node_id, name } => {
                let Some(node) =
                    self.live_handle_for_dom_frontend_node_id(inspector_session_id, node_id)
                else {
                    return Ok(RendererDomEditOutcome::NodeNotFound);
                };
                DomInspectorEdit::SetNodeName { node, name }
            }
            RendererDomEdit::SetNodeValue { node_id, value } => {
                let Some(node) =
                    self.live_handle_for_dom_frontend_node_id(inspector_session_id, node_id)
                else {
                    return Ok(RendererDomEditOutcome::NodeNotFound);
                };
                DomInspectorEdit::SetNodeValue { node, value }
            }
            RendererDomEdit::SetOuterHtml {
                node_id,
                outer_html,
            } => {
                let Some(node) =
                    self.live_handle_for_dom_frontend_node_id(inspector_session_id, node_id)
                else {
                    return Ok(RendererDomEditOutcome::NodeNotFound);
                };
                DomInspectorEdit::SetOuterHtml { node, outer_html }
            }
        };

        let outcome = self.vm_mut().edit_document_node(edit)?;
        // Structural edit events own the replacement/moved node's new frontend
        // binding. Materialize them before producing the command result so the
        // returned nodeId is the same one exposed by childNodeInserted.
        self.flush_pending_dom_mutation_event_batches();
        Ok(match outcome {
            DomInspectorEditOutcome::Applied { result_node } => {
                let result_frontend_node_id = match result_node {
                    Some(handle) => {
                        let Some(backend_node_id) =
                            self.renderer_backend_node_id_for_live_handle(handle)
                        else {
                            return Ok(RendererDomEditOutcome::MutationFailed);
                        };
                        Some(self.document_frontend_node_id_for_backend_node_id(
                            inspector_session_id,
                            backend_node_id,
                        ))
                    }
                    None => None,
                };
                RendererDomEditOutcome::Applied {
                    result_frontend_node_id,
                }
            }
            DomInspectorEditOutcome::NodeNotFound => RendererDomEditOutcome::NodeNotFound,
            DomInspectorEditOutcome::NodeNotElement => RendererDomEditOutcome::NodeNotElement,
            DomInspectorEditOutcome::NodeValueUnsupported => {
                RendererDomEditOutcome::NodeValueUnsupported
            }
            DomInspectorEditOutcome::MoveIntoSelfOrDescendant => {
                RendererDomEditOutcome::MoveIntoSelfOrDescendant
            }
            DomInspectorEditOutcome::AnchorNotChildOfTarget => {
                RendererDomEditOutcome::AnchorNotChildOfTarget
            }
            DomInspectorEditOutcome::DetachedNode => RendererDomEditOutcome::DetachedNode,
            DomInspectorEditOutcome::InvalidName { name } => {
                RendererDomEditOutcome::InvalidName { name }
            }
            DomInspectorEditOutcome::CouldNotParseAttributes => {
                RendererDomEditOutcome::CouldNotParseAttributes
            }
            DomInspectorEditOutcome::MutationFailed => RendererDomEditOutcome::MutationFailed,
        })
    }

    pub(crate) fn focus_document_node(
        &mut self,
        reference: RendererDomNodeReference,
    ) -> Result<RendererDomFocusOutcome> {
        let Some(handle) = self.live_handle_for_document_node_reference(reference) else {
            return Ok(RendererDomFocusOutcome::NodeNotFound);
        };
        self.vm_mut().focus_document_node(handle)
    }

    pub(crate) fn focus_document_node_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
    ) -> Result<RendererDomFocusOutcome> {
        let Some(handle) = self
            .vm_mut()
            .live_node_handle_for_runtime_object_id(inspector_session_id, object_id)?
        else {
            return Ok(RendererDomFocusOutcome::NodeNotFound);
        };
        self.vm_mut().focus_document_node(handle)
    }
}

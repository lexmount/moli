use super::*;
use std::collections::HashMap;

use crate::document_runtime::DomHandle;
use crate::dom::native::{DomHost, Node, NodeData, NodeType};
use crate::runtime::page_generated_dom::{
    user_agent_shadow_node_snapshot, user_agent_shadow_root_snapshot,
};
use crate::runtime::page_surface::{
    RendererAccessibilityPayloadsForObjectId, RendererDocumentBoxModel,
    RendererDocumentChildNodeSnapshotEvent, RendererDocumentChildNodeSnapshotEvents,
    RendererDocumentFrontendNodeIdsResolution, RendererDocumentHitTestResult,
    RendererDocumentNodeAttributesResolution, RendererDocumentNodeClientRect,
    RendererDocumentNodeGeometry, RendererDocumentNodePropertyResolution,
    RendererDocumentNodeReference, RendererDocumentNodeTextResolution,
    RendererDocumentQuerySelectorNode, RendererDocumentQuerySelectorResolution,
    RendererDocumentQuerySelectorWithChildNodeSnapshotEvents, RendererDomEdit,
    RendererDomEditOutcome, RendererGeometryQuad, RendererRuntimeCommandOutput,
    RendererRuntimeEvaluationResult, RendererRuntimeInspectorMessage, RendererRuntimeRemoteObject,
    RendererRuntimeRemoteObjectResolution,
};
use crate::runtime::page_vm::{
    PageVmRuntimeCommandLifecycleTarget, PageVmRuntimeCommandOutputScope,
    PageVmRuntimeCommandOutputScopeId,
};
use crate::script_vm::{
    DomInspectorEdit, DomInspectorEditOutcome, PendingRuntimeEvaluateCall,
    RuntimeEvaluateCodeGenerationPolicy, RuntimeEvaluateOutcome, RuntimeEvaluateResultMode,
};
use moli_page_types::{
    DocumentNodeAssociatedSnapshot, DocumentNodeAttributeSnapshot, DocumentNodeInspectorIdentity,
    DocumentNodeObjectSnapshot, DocumentNodeSnapshot, MAX_DOM_OUTPUT_TREE_DEPTH,
    RendererDomDebuggerDomBreakpointType, RendererDomDebuggerEventListenerBreakpoint,
    renderer_inspector_protocol_configuration_command_from_message,
};
use moli_selector::QueryEngine;

mod child_frame_nodes;
use child_frame_nodes::{
    child_frame_document_contains_live_handle, collect_shadow_including_document_handles,
};

fn script_truthy_sleep_for(
    ms_to_next: Option<u64>,
    remaining: std::time::Duration,
) -> std::time::Duration {
    ms_to_next
        .map(std::time::Duration::from_millis)
        .unwrap_or(remaining)
        .min(remaining)
}

const DOM_STABLE_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(120);
const COMMAND_WAIT_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

fn inspector_whitespace_character(character: char) -> bool {
    // Blink's WTF::IsSpaceOrNewline uses ASCII whitespace plus Unicode bidi
    // White_Space_Neutral characters. Rust's Unicode White_Space property is
    // slightly broader for these separator characters.
    character.is_whitespace()
        && !matches!(character, '\u{0085}' | '\u{00a0}' | '\u{2029}' | '\u{202f}')
}

pub(crate) fn inspector_whitespace_text_value(value: &str) -> bool {
    value.chars().all(inspector_whitespace_character)
}

pub(crate) fn inspector_whitespace_text_node(dom_host: &DomHost, handle: DomHandle) -> bool {
    matches!(
        dom_host.node(handle).map(Node::kind),
        Some(NodeData::Text(text)) if inspector_whitespace_text_value(text.data())
    )
}

pub(crate) fn inspector_whitespace_text_snapshot(snapshot: &DocumentNodeSnapshot) -> bool {
    snapshot.node_type == NodeType::Text as u8
        && inspector_whitespace_text_value(&snapshot.node_value)
}

fn child_content_document_belongs_to_top_target(
    top_document_url: &url::Url,
    child_document_url: &url::Url,
    same_origin_with_top: bool,
    has_opaque_origin: bool,
) -> bool {
    if has_opaque_origin {
        return false;
    }
    if same_origin_with_top {
        return true;
    }
    let top_site = moli_storage_key::site_for_url(top_document_url);
    top_site != "null" && top_site == moli_storage_key::site_for_url(child_document_url)
}

fn renderer_inspector_response_succeeded(
    messages: &[RendererRuntimeInspectorMessage],
    call_id: u64,
) -> bool {
    messages.iter().any(|message| {
        let RendererRuntimeInspectorMessage::Protocol(message) = message else {
            return false;
        };
        message.get("id").and_then(Value::as_u64) == Some(call_id) && message.get("error").is_none()
    })
}
const DOM_STABLE_COMPLETE_BASE_WINDOW: std::time::Duration = std::time::Duration::from_millis(350);
const DOM_STABLE_COMPLETE_RUNTIME_WINDOW: std::time::Duration =
    std::time::Duration::from_millis(1200);
const DOM_STABLE_INTERACTIVE_WINDOW: std::time::Duration = std::time::Duration::from_millis(1200);

fn runtime_inspector_response_message(
    messages: &[RendererRuntimeInspectorMessage],
    response_id: u64,
) -> Option<&Value> {
    messages.iter().find_map(|message| {
        let RendererRuntimeInspectorMessage::Protocol(message) = message else {
            return None;
        };
        (message.get("id").and_then(Value::as_u64) == Some(response_id)).then_some(message.value())
    })
}

fn dom_stable_window_for_snapshot(
    snapshot: &str,
    saw_post_domcontentloaded_runtime_work: bool,
    has_long_pending_timeout: bool,
) -> Option<std::time::Duration> {
    match snapshot.split('|').next().unwrap_or_default().trim() {
        "complete" => Some(
            if saw_post_domcontentloaded_runtime_work || has_long_pending_timeout {
                DOM_STABLE_COMPLETE_RUNTIME_WINDOW
            } else {
                DOM_STABLE_COMPLETE_BASE_WINDOW
            },
        ),
        "interactive" => Some(DOM_STABLE_INTERACTIVE_WINDOW),
        _ => None,
    }
}

fn dom_stable_sleep_for(
    ms_to_next: Option<u64>,
    stable_remaining: std::time::Duration,
    remaining: std::time::Duration,
) -> std::time::Duration {
    ms_to_next
        .map(std::time::Duration::from_millis)
        .unwrap_or(remaining)
        .min(DOM_STABLE_POLL_INTERVAL)
        .min(stable_remaining)
        .min(remaining)
}

pub(crate) fn live_document_node_snapshot(
    dom_host: &DomHost,
    node_id: DomHandle,
    depth: i32,
    parent_id: Option<DomHandle>,
    pierce: bool,
) -> Option<DocumentNodeSnapshot> {
    live_document_node_snapshot_with_budget(
        dom_host,
        node_id,
        depth,
        parent_id,
        pierce,
        true,
        false,
        MAX_DOM_OUTPUT_TREE_DEPTH,
    )
}

pub(crate) fn live_inspector_document_node_snapshot(
    dom_host: &DomHost,
    node_id: DomHandle,
    depth: i32,
    parent_id: Option<DomHandle>,
    pierce: bool,
    include_whitespace: bool,
) -> Option<DocumentNodeSnapshot> {
    live_document_node_snapshot_with_budget(
        dom_host,
        node_id,
        depth,
        parent_id,
        pierce,
        include_whitespace,
        true,
        MAX_DOM_OUTPUT_TREE_DEPTH,
    )
}

fn live_document_node_snapshot_with_budget(
    dom_host: &DomHost,
    node_id: DomHandle,
    depth: i32,
    parent_id: Option<DomHandle>,
    pierce: bool,
    include_whitespace: bool,
    force_single_text_child_at_depth_boundary: bool,
    remaining_tree_depth: usize,
) -> Option<DocumentNodeSnapshot> {
    let node = dom_host.node(node_id)?;
    let mut child_ids = dom_host.child_handles(node_id).collect::<Vec<_>>();
    let forced_single_text_child_id = if force_single_text_child_at_depth_boundary
        && depth == 0
        && let [only_child_id] = child_ids.as_slice()
        && matches!(dom_host.node(*only_child_id)?.kind(), NodeData::Text(_))
    {
        Some(*only_child_id)
    } else {
        None
    };
    child_ids
        .retain(|child| include_whitespace || !inspector_whitespace_text_node(dom_host, *child));
    let template_content_id = node
        .as_element()
        .and_then(|element| element.template_contents());
    let owner_document = dom_host
        .owner_document_handle(node_id)
        .unwrap_or_else(|| dom_host.document_handle());
    let document_url = dom_host
        .document_url_for_handle(owner_document)
        .map(url::Url::as_str)
        .unwrap_or("about:blank")
        .to_owned();
    let base_url = dom_host
        .document_base_url_for_handle(owner_document)
        .map(|url| url.as_str().to_owned())
        .unwrap_or_else(|| document_url.clone());

    let (
        node_type,
        node_name,
        local_name,
        node_value,
        namespace_uri,
        attributes,
        document_type_name,
        public_id,
        system_id,
        is_element,
        has_geometry,
    ) = match node.kind() {
        NodeData::Document(_) => (
            NodeType::Document as u8,
            "#document".to_owned(),
            String::new(),
            String::new(),
            None,
            Vec::new(),
            None,
            None,
            None,
            false,
            true,
        ),
        NodeData::DocumentType(document_type) => (
            NodeType::DocumentType as u8,
            document_type.name().to_owned(),
            String::new(),
            String::new(),
            None,
            Vec::new(),
            Some(document_type.name().to_owned()),
            Some(document_type.public_id().to_owned()),
            Some(document_type.system_id().to_owned()),
            false,
            false,
        ),
        NodeData::Element(element) => (
            NodeType::Element as u8,
            element.node_name(),
            element.local_name().to_owned(),
            String::new(),
            Some(element.namespace().to_owned()),
            element
                .attributes()
                .iter()
                .map(|attribute| DocumentNodeAttributeSnapshot {
                    local_name: attribute.name(),
                    value: attribute.value().to_owned(),
                })
                .collect(),
            None,
            None,
            None,
            true,
            true,
        ),
        NodeData::Text(text) => (
            NodeType::Text as u8,
            "#text".to_owned(),
            String::new(),
            text.data().to_owned(),
            None,
            Vec::new(),
            None,
            None,
            None,
            false,
            false,
        ),
        NodeData::CDataSection(cdata) => (
            NodeType::CDataSection as u8,
            "#cdata-section".to_owned(),
            String::new(),
            cdata.data().to_owned(),
            None,
            Vec::new(),
            None,
            None,
            None,
            false,
            false,
        ),
        NodeData::Comment(comment) => (
            NodeType::Comment as u8,
            "#comment".to_owned(),
            String::new(),
            comment.data().to_owned(),
            None,
            Vec::new(),
            None,
            None,
            None,
            false,
            false,
        ),
        NodeData::ProcessingInstruction(processing_instruction) => (
            NodeType::ProcessingInstruction as u8,
            "#processing-instruction".to_owned(),
            String::new(),
            processing_instruction.data().to_owned(),
            None,
            Vec::new(),
            None,
            None,
            None,
            false,
            false,
        ),
        NodeData::DocumentFragment(_) => (
            NodeType::DocumentFragment as u8,
            "#document-fragment".to_owned(),
            String::new(),
            String::new(),
            None,
            Vec::new(),
            None,
            None,
            None,
            false,
            false,
        ),
    };

    let next_depth = if depth > 0 { depth - 1 } else { depth };
    let next_tree_depth = remaining_tree_depth.checked_sub(1);
    let children = if depth != 0 {
        child_ids
            .iter()
            .copied()
            .filter_map(|child_id| {
                let next_tree_depth = next_tree_depth?;
                live_document_node_snapshot_with_budget(
                    dom_host,
                    child_id,
                    next_depth,
                    Some(node_id),
                    pierce,
                    include_whitespace,
                    force_single_text_child_at_depth_boundary,
                    next_tree_depth,
                )
            })
            .collect()
    } else if let Some(only_child_id) = forced_single_text_child_id
        && let Some(next_tree_depth) = next_tree_depth
    {
        live_document_node_snapshot_with_budget(
            dom_host,
            only_child_id,
            0,
            Some(node_id),
            pierce,
            include_whitespace,
            force_single_text_child_at_depth_boundary,
            next_tree_depth,
        )
        .into_iter()
        .collect()
    } else {
        Vec::new()
    };

    let shadow_root_type = dom_host.shadow_root_mode(node_id);
    let parent_id = parent_id.or_else(|| live_shadow_root_host_for_handle(dom_host, node_id));
    let shadow_roots = if pierce && depth != 0 {
        dom_host
            .shadow_root_handle(node_id)
            .and_then(|shadow_root_id| {
                live_document_node_snapshot_with_budget(
                    dom_host,
                    shadow_root_id,
                    next_depth,
                    Some(node_id),
                    pierce,
                    include_whitespace,
                    force_single_text_child_at_depth_boundary,
                    next_tree_depth?,
                )
            })
            .into_iter()
            .collect()
    } else {
        Vec::new()
    };
    let associated = template_content_id.and_then(|template_content_id| {
        Some(Box::new(DocumentNodeAssociatedSnapshot::TemplateContent(
            live_document_node_snapshot_with_budget(
                dom_host,
                template_content_id,
                0,
                None,
                pierce,
                include_whitespace,
                force_single_text_child_at_depth_boundary,
                next_tree_depth?,
            )?,
        )))
    });

    Some(DocumentNodeSnapshot {
        node_id,
        parent_id,
        inspector_identity: None,
        inspector_parent_identity: None,
        frontend_node_id: None,
        parent_frontend_node_id: None,
        backend_node_id: None,
        frame_id: None,
        node_type,
        node_name,
        local_name,
        node_value,
        child_count: child_ids.len(),
        document_url,
        base_url,
        namespace_uri,
        attributes,
        document_type_name,
        public_id,
        system_id,
        is_element,
        has_geometry,
        shadow_root_type,
        shadow_roots,
        pseudo_type: None,
        pseudo_elements: Vec::new(),
        associated,
        children,
    })
}

fn marker_pseudo_element_snapshot(
    originating_element: &DocumentNodeSnapshot,
) -> DocumentNodeSnapshot {
    DocumentNodeSnapshot {
        // Inspector-only pseudo elements use the originating element handle as
        // their renderer identity input. Backend/frontend ids are allocated
        // from the distinct pseudo-element registry key below.
        node_id: originating_element.node_id,
        parent_id: None,
        inspector_identity: Some(DocumentNodeInspectorIdentity::MarkerPseudoElement),
        inspector_parent_identity: None,
        frontend_node_id: None,
        parent_frontend_node_id: None,
        backend_node_id: None,
        frame_id: None,
        node_type: NodeType::Element as u8,
        node_name: "::marker".to_owned(),
        local_name: "::marker".to_owned(),
        node_value: String::new(),
        child_count: 0,
        document_url: originating_element.document_url.clone(),
        base_url: originating_element.base_url.clone(),
        namespace_uri: None,
        attributes: Vec::new(),
        document_type_name: None,
        public_id: None,
        system_id: None,
        is_element: true,
        has_geometry: false,
        shadow_root_type: None,
        shadow_roots: Vec::new(),
        pseudo_type: Some("marker".to_owned()),
        pseudo_elements: Vec::new(),
        associated: None,
        children: Vec::new(),
    }
}

fn live_shadow_root_host_for_handle(dom_host: &DomHost, root: DomHandle) -> Option<DomHandle> {
    dom_host.shadow_root_host(root).or_else(|| {
        let owner_document = dom_host
            .owner_document_handle(root)
            .unwrap_or_else(|| dom_host.document_handle());
        let mut stack = vec![owner_document];
        while let Some(candidate) = stack.pop() {
            if dom_host.shadow_root_handle(candidate) == Some(root) {
                return Some(candidate);
            }
            stack.extend(dom_host.child_handles(candidate));
        }
        None
    })
}

fn live_document_node_path_for_handle(dom_host: &DomHost, handle: DomHandle) -> Option<Vec<usize>> {
    let owner_document = dom_host.owner_document_handle(handle)?;
    let mut current = handle;
    let mut path = Vec::new();
    while current != owner_document {
        let parent = dom_host.dom().parent_node(current)?;
        let child_index = dom_host.child_index(parent, current)?;
        path.push(child_index);
        current = parent;
    }
    path.reverse();
    Some(path)
}

impl PageVm {
    pub(crate) fn evaluate_expression(&mut self, expression: &str) -> Result<Value> {
        self.evaluate_expression_with_await(expression, false)
    }

    pub(crate) fn evaluate_expression_with_await(
        &mut self,
        expression: &str,
        await_promise: bool,
    ) -> Result<Value> {
        self.vm_mut()
            .evaluate_expression_payload_with_await(expression, await_promise, false)
    }

    pub(crate) fn evaluate_expression_for_internal_node_reference(
        &mut self,
        handle: DomHandle,
        await_promise: bool,
        expression: impl FnOnce(u64) -> String,
    ) -> Result<Value> {
        let token = self
            .vm_mut()
            .register_internal_node_reference(handle)
            .ok_or_else(|| anyhow!("live node handle is unavailable for internal JS reference"))?;
        let expression = expression(token);
        let result = self.evaluate_expression_with_await(&expression, await_promise);
        self.vm_mut().discard_internal_node_reference(token);
        result
    }

    fn advance_runtime_evaluate(
        &mut self,
        execution_context_id: Option<i64>,
        expression: &str,
        pending_call: Option<PendingRuntimeEvaluateCall>,
        result_mode: RuntimeEvaluateResultMode,
    ) -> Result<RuntimeEvaluateOutcome> {
        if let Some(pending_call) = pending_call {
            return self.vm_mut().poll_pending_runtime_evaluate(pending_call);
        }
        self.vm_mut().begin_runtime_evaluate(
            execution_context_id,
            expression,
            true,
            false,
            None,
            RuntimeEvaluateCodeGenerationPolicy::from_cdp(None),
            result_mode,
        )
    }

    pub(crate) fn dispatch_mouse_event_at_point_with_pointer(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        button: i32,
        buttons: Option<i32>,
        click_count: i32,
        delta_x: f64,
        delta_y: f64,
        pointer: RendererPointerEventProperties,
        modifiers: u8,
    ) -> Result<RendererInputDispatchOutcome> {
        let mut outcome = self
            .vm_mut()
            .dispatch_mouse_event_at_point_with_pointer_and_modifiers(
                x,
                y,
                event_name,
                button,
                buttons,
                click_count,
                delta_x,
                delta_y,
                pointer,
                modifiers,
            )?;
        self.bind_input_dispatch_file_chooser_backend_node_id(&mut outcome);
        Ok(outcome)
    }

    pub(crate) fn dispatch_touch_event_at_points(
        &mut self,
        points: &[RendererTouchPoint],
        event_name: &str,
        activate: bool,
    ) -> Result<RendererInputDispatchOutcome> {
        let mut outcome = self
            .vm_mut()
            .dispatch_touch_event_at_points(points, event_name, activate)?;
        self.bind_input_dispatch_file_chooser_backend_node_id(&mut outcome);
        Ok(outcome)
    }

    pub(crate) fn dispatch_drag_event_at_point(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        data: RendererDragData,
        modifiers: u8,
    ) -> Result<RendererInputDispatchOutcome> {
        let mut outcome = self
            .vm_mut()
            .dispatch_drag_event_at_point(x, y, event_name, data, modifiers)?;
        self.bind_input_dispatch_file_chooser_backend_node_id(&mut outcome);
        Ok(outcome)
    }

    fn bind_input_dispatch_file_chooser_backend_node_id(
        &mut self,
        outcome: &mut RendererInputDispatchOutcome,
    ) {
        let Some(file_chooser) = outcome.pending_file_chooser.as_mut() else {
            return;
        };
        if !self.bind_file_chooser_activation_backend_node_id(file_chooser) {
            outcome.pending_file_chooser = None;
        }
    }

    pub(crate) fn clear_active_drag_data_transfer(&mut self) -> Result<()> {
        self.vm_mut().clear_active_drag_data_transfer()
    }

    pub(crate) fn set_file_input_files_for_backend_node_id(
        &mut self,
        backend_node_id: u32,
        files: Vec<crate::dom::native::SelectedFile>,
        append: bool,
    ) -> Result<Option<bool>> {
        let Some(handle) = self.live_handle_for_backend_node_id(backend_node_id) else {
            return Ok(None);
        };
        self.vm_mut()
            .set_file_input_files(handle, files, append)
            .map(Some)
    }

    pub(crate) fn set_file_input_files_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
        files: Vec<crate::dom::native::SelectedFile>,
        append: bool,
    ) -> Result<Option<bool>> {
        let Some(handle) = self
            .vm_mut()
            .live_node_handle_for_runtime_object_id(inspector_session_id, object_id)?
        else {
            return Ok(None);
        };
        self.vm_mut()
            .set_file_input_files(handle, files, append)
            .map(Some)
    }

    pub(crate) fn insert_text_into_active_control(&mut self, text: &str) -> Result<bool> {
        self.vm_mut().insert_text_into_active_control(text)
    }

    pub(crate) fn dispatch_key_event(
        &mut self,
        event_name: &str,
        key: &str,
        code: &str,
        text: &str,
        modifiers: u8,
        auto_repeat: bool,
        should_insert_text: bool,
    ) -> Result<RendererInputDispatchOutcome> {
        self.vm_mut().dispatch_key_event(
            event_name,
            key,
            code,
            text,
            modifiers,
            auto_repeat,
            should_insert_text,
        )
    }

    pub(crate) fn dispatch_runtime_protocol_message_for_inspector_session(
        &mut self,
        inspector_session_id: Option<&str>,
        raw_json: &str,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        let messages = self
            .vm_mut()
            .dispatch_inspector_protocol_message_for_session(inspector_session_id, raw_json)?;
        self.capture_runtime_inspector_v8_state(inspector_session_id);
        self.record_runtime_inspector_protocol_configuration_command(
            inspector_session_id,
            raw_json,
            &messages,
        );
        Ok(messages)
    }

    pub(crate) fn dispatch_frontend_runtime_protocol_message_for_inspector_session_with_deferred_response(
        &mut self,
        inspector_session_id: Option<&str>,
        raw_json: &str,
        deferred_response: RendererRuntimeInspectorResponseSender,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        self.dispatch_runtime_protocol_message_with_command_output(
            inspector_session_id,
            raw_json,
            deferred_response,
        )
    }

    pub(crate) fn dispatch_runtime_protocol_message_for_inspector_session_with_context_resolution(
        &mut self,
        inspector_session_id: Option<&str>,
        action: &str,
        raw_json: &str,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        if action == "evaluate"
            && let Some(messages) = self.try_dispatch_child_default_runtime_evaluate(raw_json)?
        {
            return Ok(messages);
        }
        let prepared_json =
            self.prepare_runtime_protocol_message_with_context_resolution(action, raw_json)?;
        let messages = self
            .vm_mut()
            .dispatch_inspector_protocol_message_for_session(
                inspector_session_id,
                &prepared_json,
            )?;
        self.capture_runtime_inspector_v8_state(inspector_session_id);
        self.record_runtime_inspector_protocol_configuration_command(
            inspector_session_id,
            &prepared_json,
            &messages,
        );
        Ok(messages)
    }

    pub(crate) fn dispatch_frontend_runtime_protocol_message_for_inspector_session_with_context_resolution_and_deferred_response(
        &mut self,
        inspector_session_id: Option<&str>,
        action: &str,
        raw_json: &str,
        deferred_response: RendererRuntimeInspectorResponseSender,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        if action == "evaluate"
            && let Some(messages) = self.try_dispatch_child_default_runtime_evaluate(raw_json)?
        {
            let call_id = deferred_response.call_id();
            if let Some(message) = messages.into_iter().next() {
                if deferred_response.response_delivery()
                    == moli_page_types::RendererInspectorResponseDelivery::DevToolsSession
                {
                    let deferred_response = self.vm().route_frontend_runtime_inspector_response(
                        inspector_session_id,
                        deferred_response,
                    )?;
                    let settlement = deferred_response.finalize_output(
                        RendererRuntimeCommandOutput::from_inspector_message(message),
                    );
                    self.append_runtime_command_output_settlement(settlement);
                } else if let Err(message) =
                    deferred_response.send(message.into_v8_inspector_message())
                {
                    tracing::debug!(
                        call_id,
                        message = ?message,
                        "dropping child-frame runtime response because deferred receiver was closed"
                    );
                }
            }
            return Ok(Vec::new());
        }
        let prepared_json =
            self.prepare_runtime_protocol_message_with_context_resolution(action, raw_json)?;
        self.dispatch_runtime_protocol_message_with_command_output(
            inspector_session_id,
            &prepared_json,
            deferred_response,
        )
    }

    fn dispatch_runtime_protocol_message_with_command_output(
        &mut self,
        inspector_session_id: Option<&str>,
        raw_json: &str,
        mut deferred_response: RendererRuntimeInspectorResponseSender,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        ensure!(
            self.pending_runtime_command_output.is_none(),
            "renderer runtime command output scopes cannot overlap"
        );
        ensure!(
            !self.runtime_command_settlement.has_session_response(),
            "a previous runtime session response must settle at its owner boundary before another command starts"
        );
        // A V8 Inspector response can settle after this command's physical
        // Page turn. Commit it through the Page owner so the response carries
        // the exact concrete-output tail visible at that owner boundary.
        // Otherwise the protocol scheduler could expose the response before
        // Console/lifecycle/target records produced by the same Page.
        if let Some(owner_wake) = self.runtime_hooks.owner_wake() {
            deferred_response = deferred_response.defer_publication_to_page_owner(owner_wake);
        }
        let protocol_configuration_command = serde_json::from_str::<Value>(raw_json)
            .ok()
            .and_then(|message| {
                renderer_inspector_protocol_configuration_command_from_message(&message)
            })
            .map(|(_, command)| command);
        let scope_id = PageVmRuntimeCommandOutputScopeId(self.next_runtime_command_output_scope_id);
        self.next_runtime_command_output_scope_id = self
            .next_runtime_command_output_scope_id
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("renderer runtime command output scope ID overflow"))?;
        let recorder = RendererRuntimeCommandOutputRecorder::new(
            inspector_session_id.map(str::to_owned),
            deferred_response.call_id(),
        );
        self.pending_runtime_command_output = Some(PageVmRuntimeCommandOutputScope {
            id: scope_id,
            inspector_session_id: inspector_session_id.map(str::to_owned),
            protocol_configuration_command,
            recorder: recorder.clone(),
            lifecycle_target:
                PageVmRuntimeCommandLifecycleTarget::AwaitingExplicitDocumentReplacement,
        });
        let dispatch_result = self
            .vm_mut()
            .dispatch_inspector_protocol_message_for_session_with_deferred_response_and_command_output(
                inspector_session_id,
                raw_json,
                deferred_response,
                recorder.clone(),
            );
        if let Some(state) = self.vm().inspector_v8_session_state(inspector_session_id) {
            recorder.set_v8_state_update(state);
        }
        let replacement_lifecycle_ready = dispatch_result.is_ok()
            && self
                .ready_document_replacement_lifecycle_admission()
                .is_some();
        if replacement_lifecycle_ready {
            return Ok(Vec::new());
        }

        let had_response = recorder.has_response();
        let command_error = dispatch_result.as_ref().err().map(ToString::to_string);
        self.finish_pending_runtime_command_output(command_error, dispatch_result.is_err());

        if had_response {
            return Ok(Vec::new());
        }
        dispatch_result?;
        Ok(Vec::new())
    }

    fn capture_runtime_inspector_v8_state(&mut self, inspector_session_id: Option<&str>) {
        if let Some(state) = self.vm().inspector_v8_session_state(inspector_session_id) {
            self.runtime_command_settlement.set_v8_state_update(state);
        }
    }

    pub(super) fn has_pending_runtime_command_lifecycle(&self) -> bool {
        self.pending_runtime_command_output.is_some()
    }

    pub(super) fn pending_runtime_command_output_scope_id(
        &self,
    ) -> Option<PageVmRuntimeCommandOutputScopeId> {
        self.pending_runtime_command_output
            .as_ref()
            .map(|scope| scope.id)
    }

    pub(super) fn bind_pending_runtime_command_lifecycle_observer(
        &mut self,
        document: RendererDocumentLifecycleIdentity,
    ) -> Result<()> {
        let scope = self
            .pending_runtime_command_output
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("runtime command lifecycle observer is not pending"))?;
        match scope.lifecycle_target {
            PageVmRuntimeCommandLifecycleTarget::AwaitingExplicitDocumentReplacement => {
                scope.lifecycle_target = PageVmRuntimeCommandLifecycleTarget::Exact(document);
                Ok(())
            }
            PageVmRuntimeCommandLifecycleTarget::Exact(existing) => {
                ensure!(
                    existing == document,
                    "runtime command lifecycle observer cannot be rebound to another Document"
                );
                Ok(())
            }
        }
    }

    pub(super) fn abandon_pending_runtime_command_lifecycle(
        &mut self,
        expected_scope_id: PageVmRuntimeCommandOutputScopeId,
    ) -> bool {
        if !self
            .pending_runtime_command_output
            .as_ref()
            .is_some_and(|scope| scope.id == expected_scope_id)
        {
            return false;
        }
        let Some(scope) = self.pending_runtime_command_output.take() else {
            return false;
        };
        self.vm()
            .end_runtime_inspector_command_output(scope.inspector_session_id.as_deref());
        self.vm().cancel_runtime_inspector_response_for_session(
            scope.inspector_session_id.as_deref(),
            scope.recorder.call_id(),
        );
        true
    }

    pub(super) fn finish_pending_runtime_command_output(
        &mut self,
        error_message: Option<String>,
        cancel_unparked_response: bool,
    ) -> bool {
        let Some(scope) = self.pending_runtime_command_output.take() else {
            return false;
        };
        let had_response = scope.recorder.has_response();
        self.vm()
            .end_runtime_inspector_command_output(scope.inspector_session_id.as_deref());

        if scope.recorder.response_succeeded()
            && let Some(command) = scope.protocol_configuration_command
        {
            self.apply_successful_runtime_inspector_protocol_configuration_command(
                scope.inspector_session_id.as_deref(),
                command,
            );
        }
        if cancel_unparked_response && !had_response {
            self.vm().cancel_runtime_inspector_response_for_session(
                scope.inspector_session_id.as_deref(),
                scope.recorder.call_id(),
            );
        }
        let settlement = match error_message {
            Some(message) if had_response => {
                Some(scope.recorder.finish_with_error_for_page(message))
            }
            Some(_) => {
                let _ = scope.recorder.finish();
                None
            }
            None => Some(scope.recorder.finish_for_page()),
        };
        if let Some(settlement) = settlement {
            self.append_runtime_command_output_settlement(settlement);
        }
        had_response
    }

    fn append_runtime_command_output_settlement(
        &mut self,
        settlement: super::RendererRuntimeCommandOutputSettlement,
    ) {
        self.runtime_command_settlement.append(settlement);
    }

    fn record_runtime_inspector_protocol_configuration_command(
        &mut self,
        inspector_session_id: Option<&str>,
        raw_json: &str,
        messages: &[RendererRuntimeInspectorMessage],
    ) {
        let Ok(message) = serde_json::from_str::<Value>(raw_json) else {
            return;
        };
        let Some(call_id) = message.get("id").and_then(Value::as_u64) else {
            return;
        };
        if renderer_inspector_response_succeeded(messages, call_id)
            && let Some((_call_id, command)) =
                renderer_inspector_protocol_configuration_command_from_message(&message)
        {
            self.apply_successful_runtime_inspector_protocol_configuration_command(
                inspector_session_id,
                command,
            );
        }
    }

    fn apply_successful_runtime_inspector_protocol_configuration_command(
        &mut self,
        inspector_session_id: Option<&str>,
        command: RendererInspectorProtocolConfigurationCommand,
    ) {
        let session_key = DevToolsSessionKey::from_wire_session_id(
            inspector_session_id.filter(|session_id| !session_id.is_empty()),
        );
        let remove = {
            let configuration = self
                .runtime_inspector_protocol_configurations
                .entry(session_key.clone())
                .or_default();
            configuration.apply_successful_command(command);
            !configuration.requires_restore()
        };
        if remove {
            self.runtime_inspector_protocol_configurations
                .remove(&session_key);
        }
    }

    fn runtime_inspector_frontend_restore_enabled(
        &self,
        inspector_session_id: Option<&str>,
    ) -> bool {
        let session_key = DevToolsSessionKey::from_wire_session_id(
            inspector_session_id.filter(|session_id| !session_id.is_empty()),
        );
        self.runtime_inspector_protocol_configurations
            .get(&session_key)
            .is_some_and(|configuration| configuration.runtime_frontend_enabled)
    }

    pub(crate) fn configure_dom_debugger_event_listener_breakpoint(
        &mut self,
        inspector_session_id: Option<&str>,
        breakpoint: RendererDomDebuggerEventListenerBreakpoint,
        enabled: bool,
    ) {
        self.vm_mut()
            .configure_dom_debugger_event_listener_breakpoint(
                inspector_session_id,
                breakpoint.clone(),
                enabled,
            );
        let session_key = DevToolsSessionKey::from_wire_session_id(
            inspector_session_id.filter(|session_id| !session_id.is_empty()),
        );
        let remove = {
            let configuration = self
                .runtime_inspector_protocol_configurations
                .entry(session_key.clone())
                .or_default();
            if enabled {
                configuration.set_dom_debugger_event_listener_breakpoint(breakpoint);
            } else {
                configuration.remove_dom_debugger_event_listener_breakpoint(&breakpoint);
            }
            !configuration.requires_restore()
        };
        if remove {
            self.runtime_inspector_protocol_configurations
                .remove(&session_key);
        }
    }

    pub(crate) fn configure_dom_debugger_xhr_breakpoint(
        &mut self,
        inspector_session_id: Option<&str>,
        breakpoint: RendererDomDebuggerXhrBreakpoint,
        enabled: bool,
    ) {
        self.vm_mut().configure_dom_debugger_xhr_breakpoint(
            inspector_session_id,
            breakpoint.clone(),
            enabled,
        );
        let session_key = DevToolsSessionKey::from_wire_session_id(
            inspector_session_id.filter(|session_id| !session_id.is_empty()),
        );
        let remove = {
            let configuration = self
                .runtime_inspector_protocol_configurations
                .entry(session_key.clone())
                .or_default();
            if enabled {
                configuration.set_dom_debugger_xhr_breakpoint(breakpoint);
            } else {
                configuration.remove_dom_debugger_xhr_breakpoint(&breakpoint);
            }
            !configuration.requires_restore()
        };
        if remove {
            self.runtime_inspector_protocol_configurations
                .remove(&session_key);
        }
    }

    fn try_dispatch_child_default_runtime_evaluate(
        &mut self,
        raw_json: &str,
    ) -> Result<Option<Vec<RendererRuntimeInspectorMessage>>> {
        let message: Value = serde_json::from_str(raw_json)?;
        if message.get("method").and_then(Value::as_str) != Some("Runtime.evaluate") {
            return Ok(None);
        }
        let Some(params) = message.get("params").and_then(Value::as_object) else {
            return Ok(None);
        };
        let Some(context_id) = params.get("contextId").and_then(Value::as_i64) else {
            return Ok(None);
        };
        if self
            .vm_mut()
            .child_default_frame_id_for_execution_context_id(context_id)
            .is_none()
        {
            return Ok(None);
        }
        if params.get("objectGroup").is_some()
            || params.get("generatePreview").and_then(Value::as_bool) == Some(true)
            || params.get("returnByValue").and_then(Value::as_bool) != Some(true)
        {
            return Ok(None);
        }
        let expression = params
            .get("expression")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let await_promise = params
            .get("awaitPromise")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let user_gesture = params
            .get("userGesture")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let file_prompt_handler = params
            .get(crate::script_vm::WEBDRIVER_BIDI_FILE_PROMPT_HANDLER_PARAM)
            .and_then(Value::as_str)
            .filter(|handler| matches!(*handler, "accept" | "dismiss"));
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        let result = self
            .vm_mut()
            .evaluate_expression_by_value_payload_in_context_with_await(
                Some(context_id),
                expression,
                await_promise,
                user_gesture,
                file_prompt_handler,
            );
        let response = match result {
            Ok(payload) if payload.get("exception").is_none() => {
                json!({ "id": id, "result": { "result": payload } })
            }
            Ok(payload) => {
                let description = payload
                    .get("exception")
                    .and_then(Value::as_str)
                    .unwrap_or("Error");
                json!({
                    "id": id,
                    "result": {
                        "result": {
                            "type": "object",
                            "subtype": "error",
                            "description": description,
                        },
                        "exceptionDetails": {
                            "text": "Uncaught",
                            "executionContextId": context_id,
                            "exception": {
                                "type": "object",
                                "subtype": "error",
                                "description": description,
                            },
                        },
                    },
                })
            }
            Err(error) => {
                json!({
                    "id": id,
                    "error": {
                        "code": -32000,
                        "message": error.to_string(),
                    },
                })
            }
        };
        Ok(Some(vec![RendererRuntimeInspectorMessage::protocol(
            response,
        )]))
    }

    fn prepare_runtime_protocol_message_with_context_resolution(
        &mut self,
        action: &str,
        raw_json: &str,
    ) -> Result<String> {
        let context_param_name = match action {
            "evaluate" => Some("contextId"),
            "callFunctionOn" | "addBinding" => Some("executionContextId"),
            _ => None,
        };
        let Some(context_param_name) = context_param_name else {
            return Ok(raw_json.to_owned());
        };

        let mut message: Value = serde_json::from_str(raw_json)?;
        let params = message
            .as_object_mut()
            .and_then(|message| {
                message
                    .entry("params")
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
            })
            .ok_or_else(|| anyhow!("runtime protocol params must be an object"))?;

        if let Some(context_id) = params.get(context_param_name).and_then(Value::as_i64) {
            self.rewrite_isolated_runtime_context_param(params, context_param_name, context_id)?;
        } else if action == "callFunctionOn"
            && !params.contains_key("objectId")
            && !params.contains_key("uniqueContextId")
            && let Some(context_id) = self.default_execution_context_id()
        {
            params.insert(context_param_name.to_owned(), json!(context_id));
        }

        Ok(serde_json::to_string(&message)?)
    }

    fn rewrite_isolated_runtime_context_param(
        &mut self,
        params: &mut serde_json::Map<String, Value>,
        context_param_name: &str,
        context_id: i64,
    ) -> Result<()> {
        if !self.has_isolated_execution_context_id(context_id) {
            return Ok(());
        }

        self.ensure_isolated_worlds_attached_to_inspector()?;
        if let Some(inspector_context_id) =
            self.inspector_execution_context_id_for_isolated_context(context_id)
        {
            params.insert(context_param_name.to_owned(), json!(inspector_context_id));
        }
        Ok(())
    }

    pub(crate) fn default_execution_context_id(&self) -> Option<i64> {
        self.vm().default_execution_context_id()
    }

    pub(crate) fn default_or_initial_execution_context_id(&self) -> Option<i64> {
        self.vm().default_or_initial_execution_context_id()
    }

    pub(crate) fn has_isolated_execution_context_id(&self, execution_context_id: i64) -> bool {
        self.vm()
            .has_isolated_execution_context_id(execution_context_id)
    }

    pub(crate) fn has_isolated_world_named(&self, name: &str) -> bool {
        self.vm().has_isolated_world_named(name)
    }

    pub(crate) fn has_isolated_world_named_for_frame(&self, frame_id: &str, name: &str) -> bool {
        self.vm().has_isolated_world_named_for_frame(frame_id, name)
    }

    pub(crate) fn inspector_execution_context_id_for_isolated_context(
        &self,
        execution_context_id: i64,
    ) -> Option<i64> {
        self.vm()
            .inspector_execution_context_id_for_isolated_context(execution_context_id)
    }

    pub(crate) fn isolated_execution_context_id_for_inspector_context(
        &self,
        execution_context_id: i64,
    ) -> Option<i64> {
        self.vm()
            .isolated_execution_context_id_for_inspector_context(execution_context_id)
    }

    pub(crate) fn runtime_realm_inventory(&mut self) -> Vec<RendererRuntimeRealmInfo> {
        self.vm_mut().runtime_realm_inventory()
    }

    pub(crate) fn live_child_default_runtime_realm_inventory(
        &mut self,
    ) -> Vec<RendererRuntimeRealmInfo> {
        self.vm_mut().live_child_default_runtime_realm_inventory()
    }

    pub(crate) fn ensure_isolated_worlds_attached_to_inspector(&mut self) -> Result<()> {
        self.vm_mut().ensure_isolated_worlds_attached_to_inspector()
    }

    pub(crate) fn create_isolated_world(
        &mut self,
        name: &str,
        grant_universal_access: bool,
    ) -> Result<i64> {
        self.vm_mut()
            .create_isolated_world(name, grant_universal_access)
    }

    pub(crate) fn apply_runtime_protocol_state(
        &mut self,
        inspector_session_id: Option<&str>,
        session_restore_snapshots: &[crate::runtime::RendererInspectorSessionRestoreSnapshot],
        isolated_worlds: &[crate::protocol_types::RuntimeIsolatedWorldDefinition],
        stored_runtime_bindings: &[crate::protocol_types::RuntimeBindingRegistration],
        session_runtime_bindings: &[crate::protocol_types::RuntimeBindingRegistration],
    ) -> Result<()> {
        self.vm()
            .reattach_v8_inspector_sessions(session_restore_snapshots);
        self.set_runtime_isolated_world_definitions(isolated_worlds);
        self.set_stored_runtime_bindings(stored_runtime_bindings);
        self.set_inspector_session_runtime_bindings(inspector_session_id, session_runtime_bindings);
        for world in isolated_worlds {
            self.create_isolated_world(&world.name, world.grant_universal_access)?;
        }
        for binding in session_runtime_bindings {
            self.add_runtime_binding(
                inspector_session_id,
                &binding.name,
                binding.execution_context_name.as_deref(),
                None,
            )?;
        }
        for binding in stored_runtime_bindings {
            if session_runtime_bindings.contains(binding) {
                continue;
            }
            self.install_runtime_binding(
                &binding.name,
                binding.execution_context_name.as_deref(),
                None,
            )?;
        }
        Ok(())
    }

    pub(crate) fn create_isolated_world_for_frame(
        &mut self,
        frame_id: &str,
        name: &str,
        grant_universal_access: bool,
    ) -> Result<i64> {
        self.vm_mut()
            .create_isolated_world_for_frame(frame_id, name, grant_universal_access)
    }

    fn set_runtime_isolated_world_definitions(
        &mut self,
        isolated_worlds: &[crate::protocol_types::RuntimeIsolatedWorldDefinition],
    ) {
        self.runtime_isolated_worlds.clear();
        for world in isolated_worlds {
            self.remember_runtime_isolated_world_definition(
                &world.name,
                world.grant_universal_access,
            );
        }
    }

    fn remember_runtime_isolated_world_definition(
        &mut self,
        name: &str,
        grant_universal_access: bool,
    ) {
        if let Some(existing) = self
            .runtime_isolated_worlds
            .iter_mut()
            .find(|world| world.name == name)
        {
            existing.grant_universal_access |= grant_universal_access;
            return;
        }
        self.runtime_isolated_worlds
            .push(crate::protocol_types::RuntimeIsolatedWorldDefinition {
                name: name.to_owned(),
                grant_universal_access,
            });
    }

    pub(crate) fn create_isolated_world_runtime_activity(
        &mut self,
        inspector_session_id: Option<&str>,
        frame_id: Option<&str>,
        name: &str,
        grant_universal_access: bool,
    ) -> Result<i64> {
        let had_world = match frame_id {
            Some(frame_id) => self.has_isolated_world_named_for_frame(frame_id, name),
            None => self.has_isolated_world_named(name),
        };
        let execution_context_id = match frame_id {
            Some(frame_id) => {
                self.create_isolated_world_for_frame(frame_id, name, grant_universal_access)?
            }
            None => self.create_isolated_world(name, grant_universal_access)?,
        };
        let runtime_bindings =
            self.inspector_session_runtime_bindings_for_world(inspector_session_id, name);
        for binding in runtime_bindings {
            if let Err(error) =
                self.install_runtime_binding(&binding.name, Some(name), Some(execution_context_id))
            {
                tracing::debug!(
                    %error,
                    execution_context_id,
                    world_name = name,
                    binding_name = binding.name.as_str(),
                    "isolated world binding install failed after context creation"
                );
            }
        }
        if !had_world {
            let scripts = self
                .document_start_scripts
                .iter()
                .filter(|script| script.world_name.as_deref() == Some(name))
                .cloned()
                .collect::<Vec<_>>();
            for script in scripts {
                if let Err(error) = self
                    .run_document_start_script_in_execution_context(execution_context_id, &script)
                {
                    tracing::debug!(
                        %error,
                        execution_context_id,
                        world_name = name,
                        "isolated world document-start script failed after context creation"
                    );
                }
            }
        }
        Ok(execution_context_id)
    }

    #[cfg(test)]
    pub(crate) fn take_completed_child_frame_navigation_loads(
        &mut self,
    ) -> Vec<crate::protocol_types::ChildFrameNavigationSnapshot> {
        self.vm_mut().take_completed_child_frame_navigation_loads()
    }

    #[cfg(test)]
    pub(crate) fn take_completed_child_document_networks(
        &mut self,
    ) -> Vec<crate::protocol_types::ChildFrameDocumentNetworkActivitySnapshot> {
        self.vm_mut().take_completed_child_document_networks()
    }

    #[cfg(test)]
    pub(crate) fn take_pending_child_frame_tree_events(
        &mut self,
    ) -> Vec<crate::protocol_types::ChildFrameTreeEventSnapshot> {
        self.vm_mut().take_pending_child_frame_tree_events()
    }

    pub(crate) fn evaluate_expression_in_execution_context_with_await(
        &mut self,
        execution_context_id: i64,
        expression: &str,
        await_promise: bool,
    ) -> Result<Value> {
        self.vm_mut()
            .evaluate_expression_payload_in_context_with_await(
                Some(execution_context_id),
                expression,
                await_promise,
                false,
                None,
            )
    }

    pub(crate) fn computed_style_properties_for_live_handle(
        &mut self,
        handle: DomHandle,
    ) -> Result<Option<Vec<(String, String)>>> {
        Ok(self
            .vm()
            .computed_style_properties_for_inspector_handle(handle))
    }

    pub(crate) fn computed_style_properties_for_backend_node_id(
        &mut self,
        backend_node_id: u32,
    ) -> Result<Option<Vec<(String, String)>>> {
        let Some(node_id) = self.live_handle_for_backend_node_id(backend_node_id) else {
            return Ok(None);
        };
        self.computed_style_properties_for_live_handle(node_id)
    }

    pub(crate) fn computed_style_properties_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
    ) -> Result<Option<Vec<(String, String)>>> {
        let Some(node_id) = self
            .vm_mut()
            .live_node_handle_for_runtime_object_id(inspector_session_id, object_id)?
        else {
            return Ok(None);
        };
        self.computed_style_properties_for_live_handle(node_id)
    }

    pub(in crate::runtime) async fn advance_selector_wait_turn(
        &mut self,
        selector: &str,
        remaining: std::time::Duration,
    ) -> Result<PageVmCommandWaitAdvance> {
        match self.document_query_selector_for_document(None, selector, false) {
            RendererDocumentQuerySelectorResolution::Found(nodes) => {
                if let Some(node) = nodes.into_iter().next() {
                    return Ok(PageVmCommandWaitAdvance::Completed { node });
                }
            }
            RendererDocumentQuerySelectorResolution::MissingRoot => {}
            RendererDocumentQuerySelectorResolution::InvalidSelector(message) => {
                return Err(anyhow!(
                    "wait_for_selector `{selector}` failed inside renderer: {message}"
                ));
            }
        }

        let ms_to_next = self.vm().ms_to_next_timeout();
        let sleep_for = ms_to_next
            .map(std::time::Duration::from_millis)
            .unwrap_or(remaining)
            .min(COMMAND_WAIT_POLL_INTERVAL)
            .min(remaining);
        if sleep_for.is_zero() {
            Ok(PageVmCommandWaitAdvance::Progressed)
        } else {
            Ok(PageVmCommandWaitAdvance::Waiting { sleep_for })
        }
    }

    pub(in crate::runtime) async fn advance_dom_stable_wait_turn(
        &mut self,
        mut state: PageVmDomStableWaitState,
        remaining: std::time::Duration,
    ) -> Result<PageVmDomStableWaitAdvance> {
        if self.vm().has_pending_location_navigation() {
            return Ok(PageVmDomStableWaitAdvance::TriggeredNavigation);
        }

        let evaluation = self.evaluate_expression(
            r#"(() => {
                return [
                    document.readyState || "",
                    location.href || "",
                    document.title || ""
                ].join("|");
            })()"#,
        )?;
        if let Some(message) = evaluation.get("exception").and_then(Value::as_str) {
            return Err(anyhow!(
                "domstable snapshot failed inside renderer: {message}"
            ));
        }
        let snapshot = evaluation
            .get("value")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("domstable snapshot returned a non-string payload"))?
            .to_owned();
        let serialized_document = self.vm().snapshot_live_document().serialize_document();
        let pending_subresource_requests = self.pending_subresource_request_count();
        let pending_runtime_work = self
            .vm_mut()
            .has_post_domcontentloaded_runtime_work_for_wait()
            || self.vm().has_pending_webcrypto_tasks()
            || self.vm().has_pending_opfs_tasks();
        state.saw_post_domcontentloaded_runtime_work |= pending_runtime_work;
        let has_long_pending_timeout = self
            .vm()
            .ms_to_next_timeout()
            .is_some_and(|ms| ms > DOM_STABLE_COMPLETE_BASE_WINDOW.as_millis() as u64);
        let snapshot = format!(
            "{snapshot}|html:{serialized_document}|network:{}|pending:{}|runtime:{}",
            self.vm().subresource_activity_epoch(),
            pending_subresource_requests,
            pending_runtime_work
        );

        let now = std::time::Instant::now();
        let mut stable_remaining = DOM_STABLE_POLL_INTERVAL;
        if pending_subresource_requests == 0 && !pending_runtime_work {
            if state.last_snapshot.as_deref() != Some(snapshot.as_str()) {
                state.saw_long_pending_timeout_for_observation = has_long_pending_timeout;
            } else {
                state.saw_long_pending_timeout_for_observation |= has_long_pending_timeout;
            }
            if let Some(stable_window) = dom_stable_window_for_snapshot(
                &snapshot,
                state.saw_post_domcontentloaded_runtime_work,
                state.saw_long_pending_timeout_for_observation,
            ) {
                if state.last_snapshot.as_deref() == Some(snapshot.as_str()) {
                    let since = state.stable_since.get_or_insert(now);
                    let stable_elapsed = now.saturating_duration_since(*since);
                    if stable_elapsed >= stable_window {
                        return Ok(PageVmDomStableWaitAdvance::Completed);
                    }
                    stable_remaining = stable_window.saturating_sub(stable_elapsed);
                } else {
                    // The stability timer starts when a new eligible snapshot is first observed.
                    // The next identical poll confirms that the serialized native DOM and
                    // subresource activity epoch have not changed since this observation; it does
                    // not restart the timer.
                    state.last_snapshot = Some(snapshot);
                    state.stable_since = Some(now);
                    stable_remaining = stable_window;
                }
            } else {
                state.last_snapshot = Some(snapshot);
                state.stable_since = None;
            }
        } else {
            state.saw_long_pending_timeout_for_observation = false;
            state.last_snapshot = Some(snapshot);
            state.stable_since = None;
        }

        let ms_to_next = self.vm().ms_to_next_timeout();

        let sleep_for = dom_stable_sleep_for(ms_to_next, stable_remaining, remaining);
        if sleep_for.is_zero() {
            Ok(PageVmDomStableWaitAdvance::Progressed { state })
        } else {
            Ok(PageVmDomStableWaitAdvance::Waiting { sleep_for, state })
        }
    }

    pub(in crate::runtime) async fn advance_script_truthy_wait_turn(
        &mut self,
        expression: &str,
        pending_call: Option<PendingRuntimeEvaluateCall>,
        remaining: std::time::Duration,
    ) -> Result<PageVmScriptTruthyWaitAdvance> {
        // Match the predicate convention used by browser automation APIs: a
        // callable result is the predicate, not a truthy value in its own
        // right. Keeping the invocation inside the evaluated expression also
        // lets Runtime.evaluate await a Promise returned by the predicate.
        // The source is evaluated exactly once per polling turn.
        let predicate_expression = script_truthy_predicate_expression(expression);
        let evaluation = self.advance_runtime_evaluate(
            None,
            &predicate_expression,
            pending_call,
            RuntimeEvaluateResultMode::RemoteObject,
        )?;
        let evaluation = match evaluation {
            RuntimeEvaluateOutcome::Pending(pending_call) => {
                return self
                    .advance_pending_script_truthy_wait(Some(pending_call), remaining)
                    .await;
            }
            RuntimeEvaluateOutcome::Complete(evaluation) => evaluation,
        };

        if let Some(message) = evaluation.get("exception").and_then(Value::as_str) {
            return Err(anyhow!(
                "wait_for_script_truthy `{expression}` failed inside renderer: {message}"
            ));
        }
        if evaluation_payload_is_truthy(&evaluation) {
            return Ok(PageVmScriptTruthyWaitAdvance::Completed);
        }

        self.advance_pending_script_truthy_wait(None, remaining)
            .await
    }

    async fn advance_pending_script_truthy_wait(
        &mut self,
        pending_call: Option<PendingRuntimeEvaluateCall>,
        remaining: std::time::Duration,
    ) -> Result<PageVmScriptTruthyWaitAdvance> {
        let ms_to_next = self.vm().ms_to_next_timeout();

        let sleep_for = script_truthy_sleep_for(ms_to_next, remaining);
        if sleep_for.is_zero() {
            Ok(PageVmScriptTruthyWaitAdvance::Progressed { pending_call })
        } else {
            Ok(PageVmScriptTruthyWaitAdvance::Waiting {
                sleep_for,
                pending_call,
            })
        }
    }

    pub(in crate::runtime) async fn advance_runtime_expression_await_turn(
        &mut self,
        execution_context_id: Option<i64>,
        expression: &str,
        pending_call: Option<PendingRuntimeEvaluateCall>,
        remaining: std::time::Duration,
        result_mode: RuntimeEvaluateResultMode,
    ) -> Result<PageVmRuntimeExpressionAwaitAdvance> {
        match self.advance_runtime_evaluate(
            execution_context_id,
            expression,
            pending_call,
            result_mode,
        )? {
            RuntimeEvaluateOutcome::Complete(payload) => {
                Ok(PageVmRuntimeExpressionAwaitAdvance::Completed {
                    payload: RendererRuntimeEvaluationResult::from_protocol_payload(payload),
                })
            }
            RuntimeEvaluateOutcome::Pending(pending_call) => {
                self.advance_pending_runtime_expression_await(Some(pending_call), remaining)
                    .await
            }
        }
    }

    pub(crate) fn cancel_pending_runtime_evaluate(
        &mut self,
        pending_call: Option<PendingRuntimeEvaluateCall>,
    ) {
        if let Some(pending_call) = pending_call {
            self.vm_mut().cancel_pending_runtime_evaluate(pending_call);
        }
    }

    async fn advance_pending_runtime_expression_await(
        &mut self,
        pending_call: Option<PendingRuntimeEvaluateCall>,
        remaining: std::time::Duration,
    ) -> Result<PageVmRuntimeExpressionAwaitAdvance> {
        let ms_to_next = self.vm().ms_to_next_timeout();

        let sleep_for = script_truthy_sleep_for(ms_to_next, remaining);
        if sleep_for.is_zero() {
            Ok(PageVmRuntimeExpressionAwaitAdvance::Progressed { pending_call })
        } else {
            Ok(PageVmRuntimeExpressionAwaitAdvance::Waiting {
                sleep_for,
                pending_call,
            })
        }
    }

    pub(crate) fn runtime_enable_events(
        &mut self,
        inspector_session_id: Option<&str>,
    ) -> Result<RendererRuntimeCommandOutput> {
        const INTERNAL_RUNTIME_ENABLE_ID: u64 = 900_003;

        // The renderer-owner command boundary has already waited for pending
        // exact-Document child-realm tasks. Runtime.enable can therefore use
        // V8's authoritative reportAllContexts() result directly. Do not add
        // a realm-inventory replay here: live context creation has exactly one
        // producer, while late enable is this command-local V8 report.
        let already_enabled = self.runtime_inspector_frontend_restore_enabled(inspector_session_id);
        let mut messages = if already_enabled {
            Vec::new()
        } else {
            let request = json!({
                "id": INTERNAL_RUNTIME_ENABLE_ID,
                "method": "Runtime.enable",
            });
            let raw_request = serde_json::to_string(&request)?;
            let messages = self
                .vm_mut()
                .dispatch_internal_inspector_protocol_message_for_session(
                    inspector_session_id,
                    &raw_request,
                    i32::try_from(INTERNAL_RUNTIME_ENABLE_ID)
                        .expect("bounded internal Runtime.enable call id"),
                )?;
            self.record_runtime_inspector_protocol_configuration_command(
                inspector_session_id,
                &raw_request,
                &messages,
            );
            messages
        };
        self.vm_mut()
            .ensure_isolated_worlds_attached_to_inspector()?;
        messages.extend(
            self.vm_mut()
                .take_runtime_inspector_messages(inspector_session_id),
        );
        let messages = messages
            .into_iter()
            .filter(RendererRuntimeInspectorMessage::has_v8_inspector_method)
            .collect();
        let mut output = RendererRuntimeCommandOutput::from_messages(messages);
        if let Some(state) = self.vm().inspector_v8_session_state(inspector_session_id) {
            output.set_v8_state_update(state);
        }
        Ok(output)
    }

    pub(crate) fn add_runtime_binding(
        &mut self,
        inspector_session_id: Option<&str>,
        name: &str,
        execution_context_name: Option<&str>,
        execution_context_id: Option<i64>,
    ) -> Result<()> {
        const INTERNAL_RUNTIME_ADD_BINDING_ID: u64 = 900_004;

        let inspector_execution_context_id = if let Some(execution_context_id) =
            execution_context_id
        {
            if self.has_isolated_execution_context_id(execution_context_id) {
                match self.inspector_execution_context_id_for_isolated_context(execution_context_id)
                {
                    Some(inspector_context_id) => Some(inspector_context_id),
                    None => {
                        self.install_runtime_binding(
                            name,
                            execution_context_name,
                            Some(execution_context_id),
                        )?;
                        return Ok(());
                    }
                }
            } else {
                Some(execution_context_id)
            }
        } else {
            None
        };

        let mut params = json!({ "name": name });
        if let Some(execution_context_name) = execution_context_name {
            params["executionContextName"] = json!(execution_context_name);
        }
        if let Some(inspector_execution_context_id) = inspector_execution_context_id {
            params["executionContextId"] = json!(inspector_execution_context_id);
        }
        let request = json!({
            "id": INTERNAL_RUNTIME_ADD_BINDING_ID,
            "method": "Runtime.addBinding",
            "params": params,
        });
        let raw_request = serde_json::to_string(&request)?;
        let messages = self
            .vm_mut()
            .dispatch_inspector_protocol_message_for_session(inspector_session_id, &raw_request)?;
        if let Some(response) =
            runtime_inspector_response_message(&messages, INTERNAL_RUNTIME_ADD_BINDING_ID)
            && let Some(error) = response.get("error")
        {
            return Err(anyhow!("failed to add runtime binding: {error}"));
        }
        self.install_runtime_binding(name, execution_context_name, execution_context_id)?;
        Ok(())
    }

    pub(crate) fn document_node_snapshot_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
        depth: i32,
        pierce: bool,
    ) -> Result<Option<DocumentNodeObjectSnapshot>> {
        let Some(handle) = self
            .vm_mut()
            .live_node_handle_for_runtime_object_id(inspector_session_id, object_id)?
        else {
            return Ok(None);
        };
        let snapshot = self.document_node_snapshot_for_live_handle_for_session(
            inspector_session_id,
            handle,
            depth,
            pierce,
            true,
            self.document_dom_agent_includes_whitespace(inspector_session_id),
        );
        Ok(snapshot)
    }

    pub(crate) fn document_node_snapshot_for_backend_node_id(
        &mut self,
        backend_node_id: u32,
        depth: i32,
        pierce: bool,
    ) -> Result<Option<DocumentNodeObjectSnapshot>> {
        let Some(key) = self.current_renderer_backend_node_key_for_id(backend_node_id) else {
            return Ok(None);
        };
        let snapshot = match key.inspector_identity {
            None => self.document_node_snapshot_for_live_handle(key.handle, depth, pierce),
            Some(DocumentNodeInspectorIdentity::MarkerPseudoElement) => {
                self.document_marker_pseudo_element_snapshot_for_live_host(None, key.handle)
            }
            Some(identity @ DocumentNodeInspectorIdentity::UserAgentShadowTreeNode { .. }) => self
                .document_user_agent_shadow_node_snapshot_for_live_host(
                    None, key.handle, identity, depth, true,
                ),
        };
        Ok(snapshot)
    }

    pub(crate) fn document_node_snapshot_for_backend_node_id_in_inspector_session(
        &mut self,
        inspector_session_id: Option<&str>,
        backend_node_id: u32,
        depth: i32,
        pierce: bool,
    ) -> Result<Option<DocumentNodeObjectSnapshot>> {
        let Some(key) = self.current_renderer_backend_node_key_for_id(backend_node_id) else {
            return Ok(None);
        };
        let include_whitespace = self.document_dom_agent_includes_whitespace(inspector_session_id);
        let snapshot = match key.inspector_identity {
            None => self.document_node_snapshot_for_live_handle_for_session(
                inspector_session_id,
                key.handle,
                depth,
                pierce,
                true,
                include_whitespace,
            ),
            Some(DocumentNodeInspectorIdentity::MarkerPseudoElement) => self
                .document_marker_pseudo_element_snapshot_for_live_host(
                    inspector_session_id,
                    key.handle,
                ),
            Some(identity @ DocumentNodeInspectorIdentity::UserAgentShadowTreeNode { .. }) => self
                .document_user_agent_shadow_node_snapshot_for_live_host(
                    inspector_session_id,
                    key.handle,
                    identity,
                    depth,
                    include_whitespace,
                ),
        };
        Ok(snapshot)
    }

    pub(crate) fn document_node_snapshot_for_document(
        &mut self,
        inspector_session_id: Option<&str>,
        depth: i32,
        pierce: bool,
    ) -> Option<DocumentNodeObjectSnapshot> {
        let handle = self
            .vm()
            .document_runtime
            .dom_host()
            .dom()
            .document_node_id();
        let snapshot = self.document_node_snapshot_for_live_handle_for_session(
            inspector_session_id,
            handle,
            depth,
            pierce,
            true,
            self.document_dom_agent_includes_whitespace(inspector_session_id),
        )?;
        self.mark_document_snapshot_children_requested(
            inspector_session_id,
            &snapshot.snapshot,
            depth,
        );
        Some(snapshot)
    }

    pub(crate) fn document_node_snapshots_for_dom_snapshot(
        &mut self,
        depth: i32,
        pierce: bool,
    ) -> Vec<DocumentNodeObjectSnapshot> {
        let mut snapshots = Vec::new();
        let document_handle = self
            .vm()
            .document_runtime
            .dom_host()
            .dom()
            .document_node_id();
        if let Some(root) = self.document_node_snapshot_for_live_handle_for_session(
            None,
            document_handle,
            depth,
            pierce,
            false,
            true,
        ) {
            snapshots.push(root);
        }

        let child_documents = self.vm().live_child_document_handles_in_snapshot_order();
        for (frame_id, owner_node_id, document_handle) in child_documents {
            if let Some(mut snapshot) = self.document_node_snapshot_for_live_handle_for_session(
                None,
                document_handle,
                depth,
                pierce,
                false,
                true,
            ) {
                snapshot.frame_id = Some(frame_id);
                snapshot.owner_node_id = Some(owner_node_id);
                snapshots.push(snapshot);
            }
        }
        snapshots
    }

    pub(crate) fn document_child_node_snapshot_events_for_live_handles(
        &mut self,
        inspector_session_id: Option<&str>,
        handles: &[DomHandle],
        depth: i32,
        pierce: bool,
    ) -> Option<RendererDocumentChildNodeSnapshotEvents> {
        let top_snapshot_node_id = self
            .vm()
            .document_runtime
            .dom_host()
            .dom()
            .document_node_id();
        let mut events = Vec::with_capacity(handles.len());
        for &handle in handles {
            let parent_backend_node_id = self.renderer_backend_node_id_for_live_handle(handle)?;
            let parent_frontend_node_id = self.document_frontend_node_id_for_backend_node_id(
                inspector_session_id,
                parent_backend_node_id,
            );
            let snapshots = self.live_document_child_node_snapshots_for_handle(
                inspector_session_id,
                handle,
                depth,
                pierce,
            )?;
            let child_count = snapshots.len();
            self.mark_document_node_children_requested(
                inspector_session_id,
                parent_backend_node_id,
                child_count,
            );
            for snapshot in &snapshots {
                self.mark_document_snapshot_children_requested(
                    inspector_session_id,
                    snapshot,
                    depth,
                );
            }
            events.push(RendererDocumentChildNodeSnapshotEvent {
                parent_frontend_node_id,
                snapshots,
            });
        }
        Some(RendererDocumentChildNodeSnapshotEvents {
            top_snapshot_node_id,
            events,
        })
    }

    pub(crate) fn document_child_node_snapshot_events_for_backend_node_id(
        &mut self,
        inspector_session_id: Option<&str>,
        backend_node_id: u32,
        depth: i32,
        pierce: bool,
    ) -> Option<RendererDocumentChildNodeSnapshotEvents> {
        let handle = self.live_handle_for_backend_node_id(backend_node_id)?;
        self.document_child_node_snapshot_events_for_live_handles(
            inspector_session_id,
            &[handle],
            depth,
            pierce,
        )
    }

    fn document_query_selector_node_path_snapshot_events(
        &mut self,
        inspector_session_id: Option<&str>,
        root: DomHandle,
        result_handles: &[DomHandle],
    ) -> Option<RendererDocumentChildNodeSnapshotEvents> {
        let top_snapshot_node_id = self
            .vm()
            .document_runtime
            .dom_host()
            .dom()
            .document_node_id();
        let mut events = Vec::new();

        for &result_handle in result_handles {
            let path = {
                let dom_host = self.vm().document_runtime.dom_host();
                let mut current = result_handle;
                let mut path = Vec::new();
                while current != root {
                    let parent = dom_host.dom().parent_node(current)?;
                    path.push(parent);
                    current = parent;
                }
                path.reverse();
                path
            };

            // Chromium's PushNodePathToFrontend exposes each previously
            // unrequested ancestor in root-to-leaf order. That guarantees a
            // query result is present in the frontend node map before the
            // command response returns, while shared paths are emitted once.
            for parent in path {
                let parent_backend_node_id =
                    self.renderer_backend_node_id_for_live_handle(parent)?;
                if self
                    .document_node_children_requested(inspector_session_id, parent_backend_node_id)
                {
                    continue;
                }
                let mut parent_events = self.document_child_node_snapshot_events_for_live_handles(
                    inspector_session_id,
                    &[parent],
                    0,
                    false,
                )?;
                events.append(&mut parent_events.events);
            }
        }

        Some(RendererDocumentChildNodeSnapshotEvents {
            top_snapshot_node_id,
            events,
        })
    }

    pub(crate) fn document_query_selector_for_document(
        &mut self,
        inspector_session_id: Option<&str>,
        selector: &str,
        multiple: bool,
    ) -> RendererDocumentQuerySelectorResolution {
        let root = self
            .vm()
            .document_runtime
            .dom_host()
            .dom()
            .document_node_id();
        self.document_query_selector_for_live_root_handle(
            inspector_session_id,
            root,
            selector,
            multiple,
        )
    }

    pub(crate) fn document_query_selector_for_live_root_handle(
        &mut self,
        inspector_session_id: Option<&str>,
        root: DomHandle,
        selector: &str,
        multiple: bool,
    ) -> RendererDocumentQuerySelectorResolution {
        let node_ids = {
            let dom_host = self.vm().document_runtime.dom_host();
            if dom_host.node(root).is_none() {
                return RendererDocumentQuerySelectorResolution::MissingRoot;
            }

            let engine = QueryEngine;
            let result = if root == dom_host.dom().document_node_id() {
                if multiple {
                    engine.query_selector_all_host(dom_host, selector)
                } else {
                    engine
                        .query_selector_host(dom_host, selector)
                        .map(|node_id| node_id.into_iter().collect())
                }
            } else if multiple {
                engine.query_selector_all_in_host(dom_host, root, selector)
            } else {
                engine
                    .query_selector_in_host(dom_host, root, selector)
                    .map(|node_id| node_id.into_iter().collect())
            };

            match result {
                Ok(node_ids) => node_ids,
                Err(error) => {
                    return RendererDocumentQuerySelectorResolution::InvalidSelector(
                        error.to_string(),
                    );
                }
            }
        };

        RendererDocumentQuerySelectorResolution::Found(
            self.query_selector_nodes_for_handles(inspector_session_id, node_ids),
        )
    }

    fn query_selector_nodes_for_handles(
        &mut self,
        inspector_session_id: Option<&str>,
        handles: Vec<DomHandle>,
    ) -> Vec<RendererDocumentQuerySelectorNode> {
        handles
            .into_iter()
            .filter_map(|handle| {
                let backend_node_id = self.renderer_backend_node_id_for_live_handle(handle)?;
                let frontend_node_id = self.document_frontend_node_id_for_backend_node_id(
                    inspector_session_id,
                    backend_node_id,
                );
                Some(RendererDocumentQuerySelectorNode {
                    live_node_id: handle,
                    frontend_node_id,
                    backend_node_id,
                })
            })
            .collect()
    }

    pub(crate) fn document_query_selector_for_backend_node_id(
        &mut self,
        inspector_session_id: Option<&str>,
        root_backend_node_id: u32,
        selector: &str,
        multiple: bool,
    ) -> RendererDocumentQuerySelectorResolution {
        let Some(root) = self.live_handle_for_backend_node_id(root_backend_node_id) else {
            return RendererDocumentQuerySelectorResolution::MissingRoot;
        };
        self.document_query_selector_for_live_root_handle(
            inspector_session_id,
            root,
            selector,
            multiple,
        )
    }

    pub(crate) fn document_query_selector_for_child_frame_backend_node_id(
        &mut self,
        inspector_session_id: Option<&str>,
        frame_id: &str,
        root_backend_node_id: u32,
        selector: &str,
        multiple: bool,
    ) -> RendererDocumentQuerySelectorResolution {
        let Some(document_handle) = self
            .vm()
            .child_browsing_context_document_handle_by_frame_id(frame_id)
        else {
            return RendererDocumentQuerySelectorResolution::MissingRoot;
        };
        let Some(root) = self.live_handle_for_backend_node_id(root_backend_node_id) else {
            return RendererDocumentQuerySelectorResolution::MissingRoot;
        };
        if !child_frame_document_contains_live_handle(
            self.vm().document_runtime.dom_host(),
            document_handle,
            root,
        ) {
            return RendererDocumentQuerySelectorResolution::MissingRoot;
        }

        self.document_query_selector_for_live_root_handle(
            inspector_session_id,
            root,
            selector,
            multiple,
        )
    }

    pub(crate) fn document_query_selector_with_child_node_snapshot_events_for_live_root_handle(
        &mut self,
        inspector_session_id: Option<&str>,
        root: DomHandle,
        selector: &str,
        multiple: bool,
    ) -> RendererDocumentQuerySelectorWithChildNodeSnapshotEvents {
        let query_selector_resolution = self.document_query_selector_for_live_root_handle(
            inspector_session_id,
            root,
            selector,
            multiple,
        );
        let result_handles = match &query_selector_resolution {
            RendererDocumentQuerySelectorResolution::Found(nodes) => nodes
                .iter()
                .map(|node| node.live_node_id)
                .collect::<Vec<_>>(),
            RendererDocumentQuerySelectorResolution::MissingRoot
            | RendererDocumentQuerySelectorResolution::InvalidSelector(_) => Vec::new(),
        };
        let child_node_snapshot_events = self.document_query_selector_node_path_snapshot_events(
            inspector_session_id,
            root,
            &result_handles,
        );
        RendererDocumentQuerySelectorWithChildNodeSnapshotEvents {
            child_node_snapshot_events,
            query_selector_resolution,
        }
    }

    pub(crate) fn document_query_selector_with_child_node_snapshot_events_for_backend_node_id(
        &mut self,
        inspector_session_id: Option<&str>,
        root_backend_node_id: u32,
        selector: &str,
        multiple: bool,
    ) -> RendererDocumentQuerySelectorWithChildNodeSnapshotEvents {
        let Some(root) = self.live_handle_for_backend_node_id(root_backend_node_id) else {
            return RendererDocumentQuerySelectorWithChildNodeSnapshotEvents {
                child_node_snapshot_events: None,
                query_selector_resolution: RendererDocumentQuerySelectorResolution::MissingRoot,
            };
        };
        self.document_query_selector_with_child_node_snapshot_events_for_live_root_handle(
            inspector_session_id,
            root,
            selector,
            multiple,
        )
    }

    fn live_document_child_node_snapshots_for_handle(
        &mut self,
        inspector_session_id: Option<&str>,
        handle: DomHandle,
        depth: i32,
        pierce: bool,
    ) -> Option<Vec<DocumentNodeSnapshot>> {
        let include_whitespace = self.document_dom_agent_includes_whitespace(inspector_session_id);
        let child_handles = {
            let dom_host = self.vm().document_runtime.dom_host();
            dom_host.node(handle)?;
            dom_host
                .child_handles(handle)
                .filter(|child| {
                    include_whitespace || !inspector_whitespace_text_node(dom_host, *child)
                })
                .collect::<Vec<_>>()
        };
        let mut snapshots = Vec::with_capacity(child_handles.len());
        for child_handle in child_handles {
            if let Some(mut snapshot) = live_inspector_document_node_snapshot(
                self.vm().document_runtime.dom_host(),
                child_handle,
                depth,
                Some(handle),
                pierce,
                include_whitespace,
            ) {
                self.project_generated_user_agent_shadow_roots(
                    &mut snapshot,
                    depth,
                    pierce,
                    include_whitespace,
                );
                self.project_generated_pseudo_elements(&mut snapshot);
                self.assign_renderer_dom_agent_ids_to_snapshot(
                    inspector_session_id,
                    &mut snapshot,
                    include_whitespace,
                );
                snapshots.push(snapshot);
            }
        }
        Some(snapshots)
    }

    fn document_node_snapshot_for_live_handle(
        &mut self,
        handle: DomHandle,
        depth: i32,
        pierce: bool,
    ) -> Option<DocumentNodeObjectSnapshot> {
        self.document_node_snapshot_for_live_handle_for_session(
            None, handle, depth, pierce, true, true,
        )
    }

    fn document_node_snapshot_for_live_handle_for_session(
        &mut self,
        inspector_session_id: Option<&str>,
        handle: DomHandle,
        depth: i32,
        pierce: bool,
        include_generated_user_agent_shadow_roots: bool,
        include_whitespace: bool,
    ) -> Option<DocumentNodeObjectSnapshot> {
        let frame_id = self.vm().child_frame_id_for_live_node_handle(handle);
        let node_path =
            live_document_node_path_for_handle(self.vm().document_runtime.dom_host(), handle);
        let mut snapshot = live_inspector_document_node_snapshot(
            self.vm().document_runtime.dom_host(),
            handle,
            depth,
            None,
            pierce,
            include_whitespace,
        );
        if let Some(snapshot) = snapshot.as_mut() {
            self.project_child_frame_content_documents(snapshot, depth, pierce, include_whitespace);
            if include_generated_user_agent_shadow_roots {
                self.project_generated_user_agent_shadow_roots(
                    snapshot,
                    depth,
                    pierce,
                    include_whitespace,
                );
            }
            self.project_generated_pseudo_elements(snapshot);
            self.assign_renderer_dom_agent_ids_to_snapshot(
                inspector_session_id,
                snapshot,
                include_whitespace,
            );
        }
        snapshot.map(|snapshot| DocumentNodeObjectSnapshot {
            frame_id,
            owner_node_id: None,
            node_path,
            snapshot,
        })
    }

    fn document_marker_pseudo_element_snapshot_for_live_host(
        &mut self,
        inspector_session_id: Option<&str>,
        host: DomHandle,
    ) -> Option<DocumentNodeObjectSnapshot> {
        if !self
            .vm()
            .marker_pseudo_element_is_generated_for_document_snapshot(host)
        {
            return None;
        }
        let frame_id = self.vm().child_frame_id_for_live_node_handle(host);
        let originating_element = live_document_node_snapshot(
            self.vm().document_runtime.dom_host(),
            host,
            0,
            None,
            false,
        )?;
        let mut snapshot = marker_pseudo_element_snapshot(&originating_element);
        self.assign_renderer_dom_agent_ids_to_snapshot(inspector_session_id, &mut snapshot, true);
        Some(DocumentNodeObjectSnapshot {
            frame_id,
            owner_node_id: None,
            node_path: None,
            snapshot,
        })
    }

    fn document_user_agent_shadow_node_snapshot_for_live_host(
        &mut self,
        inspector_session_id: Option<&str>,
        host: DomHandle,
        identity: DocumentNodeInspectorIdentity,
        depth: i32,
        include_whitespace: bool,
    ) -> Option<DocumentNodeObjectSnapshot> {
        let frame_id = self.vm().child_frame_id_for_live_node_handle(host);
        let originating_element = live_document_node_snapshot(
            self.vm().document_runtime.dom_host(),
            host,
            0,
            None,
            false,
        )?;
        let mut snapshot = user_agent_shadow_node_snapshot(
            self.vm().document_runtime.dom_host(),
            &originating_element,
            identity,
            depth,
            include_whitespace,
        )?;
        self.assign_renderer_dom_agent_ids_to_snapshot(
            inspector_session_id,
            &mut snapshot,
            include_whitespace,
        );
        Some(DocumentNodeObjectSnapshot {
            frame_id,
            owner_node_id: None,
            node_path: None,
            snapshot,
        })
    }

    fn project_child_frame_content_documents(
        &self,
        snapshot: &mut DocumentNodeSnapshot,
        depth: i32,
        pierce: bool,
        include_whitespace: bool,
    ) {
        if !pierce || depth == 0 {
            return;
        }
        let child_documents = self
            .vm()
            .live_child_document_handles_in_snapshot_order()
            .into_iter()
            .filter(|(_, owner_node_id, _)| {
                let vm = self.vm();
                let Some(child_document_url) =
                    vm.child_browsing_context_current_url(*owner_node_id)
                else {
                    return false;
                };
                child_content_document_belongs_to_top_target(
                    vm.document_runtime.document_url(),
                    &child_document_url,
                    vm.child_browsing_context_is_same_origin_with_top(*owner_node_id),
                    vm.child_browsing_context_has_opaque_origin(*owner_node_id),
                )
            })
            .map(|(_, owner_node_id, document_handle)| (owner_node_id, document_handle))
            .collect::<HashMap<_, _>>();
        if child_documents.is_empty() {
            return;
        }

        let dom_host = self.vm().document_runtime.dom_host();
        let mut stack = vec![(snapshot, depth)];
        while let Some((snapshot, depth)) = stack.pop() {
            let next_depth = if depth > 0 { depth - 1 } else { depth };
            if depth != 0
                && snapshot.is_element
                && snapshot.inspector_identity.is_none()
                && let Some(document_handle) = child_documents.get(&snapshot.node_id)
            {
                snapshot.associated = live_inspector_document_node_snapshot(
                    dom_host,
                    *document_handle,
                    next_depth,
                    None,
                    pierce,
                    include_whitespace,
                )
                .map(DocumentNodeAssociatedSnapshot::ContentDocument)
                .map(Box::new);
            }
            for pseudo_element in snapshot.pseudo_elements.iter_mut().rev() {
                stack.push((pseudo_element, next_depth));
            }
            for shadow_root in snapshot.shadow_roots.iter_mut().rev() {
                stack.push((shadow_root, next_depth));
            }
            if let Some(associated) = snapshot.associated.as_deref_mut() {
                stack.push((associated.node_mut(), next_depth));
            }
            for child in snapshot.children.iter_mut().rev() {
                stack.push((child, next_depth));
            }
        }
    }

    fn project_generated_user_agent_shadow_roots(
        &self,
        snapshot: &mut DocumentNodeSnapshot,
        depth: i32,
        pierce: bool,
        include_whitespace: bool,
    ) {
        let mut stack = vec![(snapshot, depth)];
        while let Some((snapshot, depth)) = stack.pop() {
            let next_depth = if depth > 0 { depth - 1 } else { depth };
            if snapshot.is_element
                && snapshot.inspector_identity.is_none()
                && let Some(shadow_root) = user_agent_shadow_root_snapshot(
                    self.vm().document_runtime.dom_host(),
                    snapshot,
                    if pierce && depth != 0 { next_depth } else { 0 },
                    include_whitespace,
                )
            {
                snapshot.shadow_roots.push(shadow_root);
            }
            for pseudo_element in snapshot.pseudo_elements.iter_mut().rev() {
                stack.push((pseudo_element, next_depth));
            }
            for shadow_root in snapshot.shadow_roots.iter_mut().rev() {
                stack.push((shadow_root, next_depth));
            }
            if let Some(associated) = snapshot.associated.as_deref_mut() {
                stack.push((associated.node_mut(), next_depth));
            }
            for child in snapshot.children.iter_mut().rev() {
                stack.push((child, next_depth));
            }
        }
    }

    fn project_generated_pseudo_elements(&self, snapshot: &mut DocumentNodeSnapshot) {
        let mut stack = vec![snapshot];
        while let Some(snapshot) = stack.pop() {
            if snapshot.is_element
                && snapshot.inspector_identity.is_none()
                && self
                    .vm()
                    .marker_pseudo_element_is_generated_for_document_snapshot(snapshot.node_id)
            {
                snapshot
                    .pseudo_elements
                    .push(marker_pseudo_element_snapshot(snapshot));
            }
            for pseudo_element in snapshot.pseudo_elements.iter_mut().rev() {
                stack.push(pseudo_element);
            }
            for shadow_root in snapshot.shadow_roots.iter_mut().rev() {
                stack.push(shadow_root);
            }
            if let Some(associated) = snapshot.associated.as_deref_mut() {
                stack.push(associated.node_mut());
            }
            for child in snapshot.children.iter_mut().rev() {
                stack.push(child);
            }
        }
    }

    pub(super) fn assign_renderer_dom_agent_ids_to_snapshot(
        &mut self,
        inspector_session_id: Option<&str>,
        snapshot: &mut DocumentNodeSnapshot,
        include_whitespace: bool,
    ) {
        let mut stack = vec![snapshot];
        while let Some(snapshot) = stack.pop() {
            let backend_node_id = if let Some(inspector_identity) = snapshot.inspector_identity {
                self.renderer_backend_node_id_for_inspector_node(
                    snapshot.node_id,
                    inspector_identity,
                )
            } else {
                self.renderer_backend_node_id_for_live_handle(snapshot.node_id)
            };
            if let Some(backend_node_id) = backend_node_id {
                let frontend_node_id = self
                    .document_frontend_node_id_for_backend_node_id_in_whitespace_projection(
                        inspector_session_id,
                        backend_node_id,
                        include_whitespace,
                        inspector_whitespace_text_snapshot(snapshot),
                    );
                snapshot.backend_node_id = Some(backend_node_id);
                snapshot.frontend_node_id = Some(frontend_node_id);
            }
            snapshot.frame_id = self
                .vm()
                .child_browsing_context_frame_id_by_owner_node_id(snapshot.node_id);
            snapshot.parent_frontend_node_id = if let Some(parent_identity) =
                snapshot.inspector_parent_identity
            {
                self.renderer_backend_node_id_for_inspector_node(snapshot.node_id, parent_identity)
                    .map(|parent_backend_node_id| {
                        self.document_frontend_node_id_for_backend_node_id(
                            inspector_session_id,
                            parent_backend_node_id,
                        )
                    })
            } else {
                snapshot.parent_id.and_then(|parent_id| {
                    self.renderer_backend_node_id_for_live_handle(parent_id)
                        .map(|parent_backend_node_id| {
                            self.document_frontend_node_id_for_backend_node_id(
                                inspector_session_id,
                                parent_backend_node_id,
                            )
                        })
                })
            };
            for pseudo_element in snapshot.pseudo_elements.iter_mut().rev() {
                stack.push(pseudo_element);
            }
            for shadow_root in snapshot.shadow_roots.iter_mut().rev() {
                stack.push(shadow_root);
            }
            if let Some(associated) = snapshot.associated.as_deref_mut() {
                stack.push(associated.node_mut());
            }
            for child in snapshot.children.iter_mut().rev() {
                stack.push(child);
            }
        }
    }

    pub(crate) fn document_node_attributes_for_live_handle(
        &self,
        handle: DomHandle,
    ) -> RendererDocumentNodeAttributesResolution {
        let document = self.vm().document_runtime.dom_host().dom();
        let Some(node) = document.node(handle) else {
            return RendererDocumentNodeAttributesResolution::MissingNode;
        };
        let Some(element) = node.as_element() else {
            return RendererDocumentNodeAttributesResolution::NotElement;
        };
        let attributes = element
            .attributes()
            .iter()
            .map(|attribute| (attribute.name(), attribute.value().to_owned()))
            .collect();
        RendererDocumentNodeAttributesResolution::Found(attributes)
    }

    pub(crate) fn document_node_attributes_for_backend_node_id(
        &mut self,
        backend_node_id: u32,
    ) -> RendererDocumentNodeAttributesResolution {
        let Some(handle) = self.live_handle_for_backend_node_id(backend_node_id) else {
            return RendererDocumentNodeAttributesResolution::MissingNode;
        };
        self.document_node_attributes_for_live_handle(handle)
    }

    pub(crate) fn document_node_text_for_live_handle(
        &self,
        handle: DomHandle,
    ) -> RendererDocumentNodeTextResolution {
        let document = self.vm().document_runtime.dom_host().dom();
        match document.text_content(handle) {
            Some(text) => RendererDocumentNodeTextResolution::Found(text),
            None => RendererDocumentNodeTextResolution::MissingNode,
        }
    }

    pub(crate) fn document_node_text_for_backend_node_id(
        &mut self,
        backend_node_id: u32,
    ) -> RendererDocumentNodeTextResolution {
        let Some(handle) = self.live_handle_for_backend_node_id(backend_node_id) else {
            return RendererDocumentNodeTextResolution::MissingNode;
        };
        self.document_node_text_for_live_handle(handle)
    }

    pub(crate) fn document_node_property_for_live_handle(
        &self,
        handle: DomHandle,
        name: &str,
    ) -> RendererDocumentNodePropertyResolution {
        let document = self.vm().document_runtime.dom_host().dom();
        let Some(node) = document.node(handle) else {
            return RendererDocumentNodePropertyResolution::MissingNode;
        };
        let Some(element) = node.as_element() else {
            return RendererDocumentNodePropertyResolution::NotElement;
        };

        let value = match name {
            "tagName" => json!(element.node_name()),
            "nodeName" => json!(node.node_name()),
            "localName" => json!(element.local_name()),
            "namespaceURI" => json!(element.namespace()),
            "id" => json!(element.attribute("id").unwrap_or_default()),
            "className" => json!(element.attribute("class").unwrap_or_default()),
            "name" => json!(element.attribute("name").unwrap_or_default()),
            "title" => json!(element.attribute("title").unwrap_or_default()),
            "role" => json!(element.attribute("role").unwrap_or_default()),
            "textContent" | "innerText" => json!(document.text_content(handle).unwrap_or_default()),
            "innerHTML" => json!(document.inner_html(handle).unwrap_or_default()),
            "outerHTML" => json!(document.outer_html(handle).unwrap_or_default()),
            "value" if element.is_html_input() || element.is_html_textarea() => {
                json!(element.input_value())
            }
            "value" if element.is_html_option() => json!(element.option_value(document, handle)),
            "value" if element.is_html_select() => json!(
                document
                    .select_selected_option_elements(handle)
                    .into_iter()
                    .next()
                    .and_then(|option_id| document
                        .node(option_id)
                        .and_then(|node| node.as_element())
                        .map(|option| option.option_value(document, option_id)))
                    .unwrap_or_default()
            ),
            "checked" if element.is_html_input() => json!(element.checked()),
            "selected" if element.is_html_option() => {
                json!(document.option_effectively_selected(handle))
            }
            "disabled" => json!(element.has_attribute("disabled")),
            "readOnly" => json!(element.has_attribute("readonly")),
            "multiple" => json!(element.has_attribute("multiple")),
            "href" | "src" | "type" | "alt" | "placeholder" => {
                json!(element.attribute(name).unwrap_or_default())
            }
            "htmlFor" => json!(element.attribute("for").unwrap_or_default()),
            _ => Value::Null,
        };

        RendererDocumentNodePropertyResolution::Found(value)
    }

    pub(crate) fn document_node_property_for_backend_node_id(
        &mut self,
        backend_node_id: u32,
        name: &str,
    ) -> RendererDocumentNodePropertyResolution {
        let Some(handle) = self.live_handle_for_backend_node_id(backend_node_id) else {
            return RendererDocumentNodePropertyResolution::MissingNode;
        };
        self.document_node_property_for_live_handle(handle, name)
    }

    fn renderer_backend_node_id_map_for_document_handle(
        &mut self,
        document_handle: DomHandle,
    ) -> Option<HashMap<DomHandle, u32>> {
        let handles = {
            let dom_host = self.vm().document_runtime.dom_host();
            collect_shadow_including_document_handles(dom_host, document_handle)
        };
        Some(
            handles
                .into_iter()
                .filter_map(|handle| {
                    self.renderer_backend_node_id_for_live_handle(handle)
                        .map(|backend_node_id| (handle, backend_node_id))
                })
                .collect(),
        )
    }

    fn renderer_backend_node_id_map_for_owner_document(
        &mut self,
        handle: DomHandle,
    ) -> Option<HashMap<DomHandle, u32>> {
        let document_handle = {
            let dom_host = self.vm().document_runtime.dom_host();
            let handle = live_shadow_root_host_for_handle(dom_host, handle).unwrap_or(handle);
            dom_host.owner_document_handle(handle)?
        };
        self.renderer_backend_node_id_map_for_document_handle(document_handle)
    }

    pub(crate) fn accessibility_tree_payloads_for_live_handle(
        &mut self,
        root_handle: DomHandle,
        max_depth: Option<i32>,
    ) -> Option<Vec<serde_json::Value>> {
        let backend_node_ids = self.renderer_backend_node_id_map_for_owner_document(root_handle)?;
        let payloads = {
            let mut backend_node_id_for_node = |node_id| backend_node_ids.get(&node_id).copied();
            let document = self.vm().document_runtime.dom_host().dom();
            document.node(root_handle)?;
            moli_dom::accessibility::accessibility_tree_payloads_for_document_with_backend_node_ids(
                document,
                root_handle,
                max_depth,
                &mut backend_node_id_for_node,
            )
        }?;
        Some(payloads)
    }

    pub(crate) fn accessibility_tree_payloads_for_document(
        &mut self,
        max_depth: Option<i32>,
    ) -> Option<Vec<serde_json::Value>> {
        let root_node_id = self
            .vm()
            .document_runtime
            .dom_host()
            .dom()
            .document_node_id();
        self.accessibility_tree_payloads_for_live_handle(root_node_id, max_depth)
    }

    pub(crate) fn accessibility_node_payload_for_live_handle(
        &mut self,
        handle: DomHandle,
    ) -> Option<serde_json::Value> {
        let backend_node_ids = self.renderer_backend_node_id_map_for_owner_document(handle)?;
        let payload = {
            let mut backend_node_id_for_node = |node_id| backend_node_ids.get(&node_id).copied();
            let document = self.vm().document_runtime.dom_host().dom();
            moli_dom::accessibility::accessibility_node_payload_for_document_with_backend_node_ids(
                document,
                handle,
                &mut backend_node_id_for_node,
            )?
        };
        Some(payload)
    }

    pub(crate) fn accessibility_node_payload_for_document(&mut self) -> Option<serde_json::Value> {
        let node_id = self
            .vm()
            .document_runtime
            .dom_host()
            .dom()
            .document_node_id();
        self.accessibility_node_payload_for_live_handle(node_id)
    }

    pub(crate) fn accessibility_tree_payloads_for_backend_node_id(
        &mut self,
        backend_node_id: u32,
        max_depth: Option<i32>,
    ) -> Option<RendererAccessibilityPayloadsForObjectId> {
        self.accessibility_payloads_for_backend_node_id(
            backend_node_id,
            |document, node_id, backend_node_ids| {
                document.node(node_id)?;
                let mut backend_node_id_for_node =
                    |node_id| backend_node_ids.get(&node_id).copied();
                moli_dom::accessibility::accessibility_tree_payloads_for_document_with_backend_node_ids(
                    document,
                    node_id,
                    max_depth,
                    &mut backend_node_id_for_node,
                )
            },
        )
    }

    pub(crate) fn accessibility_node_payload_for_backend_node_id(
        &mut self,
        backend_node_id: u32,
    ) -> Option<RendererAccessibilityPayloadsForObjectId> {
        self.accessibility_payloads_for_backend_node_id(
            backend_node_id,
            |document, node_id, backend_node_ids| {
                let mut backend_node_id_for_node =
                    |node_id| backend_node_ids.get(&node_id).copied();
                moli_dom::accessibility::accessibility_node_payload_for_document_with_backend_node_ids(
                    document,
                    node_id,
                    &mut backend_node_id_for_node,
                )
                .map(|payload| vec![payload])
            },
        )
    }

    pub(crate) fn accessibility_node_and_ancestor_payloads_for_backend_node_id(
        &mut self,
        backend_node_id: u32,
    ) -> Option<RendererAccessibilityPayloadsForObjectId> {
        self.accessibility_payloads_for_backend_node_id(
            backend_node_id,
            |document, node_id, backend_node_ids| {
                document.node(node_id)?;
                let mut backend_node_id_for_node =
                    |node_id| backend_node_ids.get(&node_id).copied();
                moli_dom::accessibility::accessibility_node_and_ancestor_payloads_for_document_with_backend_node_ids(
                    document,
                    node_id,
                    &mut backend_node_id_for_node,
                )
            },
        )
    }

    pub(crate) fn accessibility_child_node_payloads_for_backend_node_id(
        &mut self,
        backend_node_id: u32,
    ) -> Option<RendererAccessibilityPayloadsForObjectId> {
        self.accessibility_payloads_for_backend_node_id(
            backend_node_id,
            |document, node_id, backend_node_ids| {
                document.node(node_id)?;
                let mut backend_node_id_for_node =
                    |node_id| backend_node_ids.get(&node_id).copied();
                moli_dom::accessibility::accessibility_child_node_payloads_for_document_with_backend_node_ids(
                    document,
                    node_id,
                    &mut backend_node_id_for_node,
                )
            },
        )
    }

    pub(crate) fn accessibility_partial_tree_payloads_for_backend_node_id(
        &mut self,
        backend_node_id: u32,
        fetch_relatives: bool,
    ) -> Option<RendererAccessibilityPayloadsForObjectId> {
        self.accessibility_payloads_for_backend_node_id(
            backend_node_id,
            |document, node_id, backend_node_ids| {
                let mut backend_node_id_for_node =
                    |node_id| backend_node_ids.get(&node_id).copied();
                moli_dom::accessibility::accessibility_partial_tree_payloads_for_document_with_backend_node_ids(
                    document,
                    node_id,
                    fetch_relatives,
                    &mut backend_node_id_for_node,
                )
            },
        )
    }

    fn accessibility_payloads_for_backend_node_id(
        &mut self,
        backend_node_id: u32,
        build_payloads: impl FnOnce(
            &crate::dom::native::NativeDom,
            DomHandle,
            &HashMap<DomHandle, u32>,
        ) -> Option<Vec<serde_json::Value>>,
    ) -> Option<RendererAccessibilityPayloadsForObjectId> {
        let handle = self.live_handle_for_backend_node_id(backend_node_id)?;
        self.accessibility_payloads_for_live_handle(handle, build_payloads)
    }

    pub(crate) fn accessibility_tree_payloads_for_child_frame(
        &mut self,
        frame_id: &str,
        max_depth: Option<i32>,
    ) -> Option<RendererAccessibilityPayloadsForObjectId> {
        let document_handle = self
            .vm()
            .child_browsing_context_document_handle_by_frame_id(frame_id)?;
        let backend_node_ids =
            self.renderer_backend_node_id_map_for_document_handle(document_handle)?;
        let payloads = {
            let mut backend_node_id_for_node = |node_id| backend_node_ids.get(&node_id).copied();
            let document = self.vm().document_runtime.dom_host().dom();
            document.node(document_handle)?;
            moli_dom::accessibility::accessibility_tree_payloads_for_document_with_backend_node_ids(
                document,
                document_handle,
                max_depth,
                &mut backend_node_id_for_node,
            )
        };
        Some(RendererAccessibilityPayloadsForObjectId {
            frame_id: Some(frame_id.to_owned()),
            payloads,
        })
    }

    pub(crate) fn accessibility_node_payload_for_child_frame(
        &mut self,
        frame_id: &str,
    ) -> Option<RendererAccessibilityPayloadsForObjectId> {
        let document_handle = self
            .vm()
            .child_browsing_context_document_handle_by_frame_id(frame_id)?;
        let backend_node_ids =
            self.renderer_backend_node_id_map_for_document_handle(document_handle)?;
        let payload = {
            let mut backend_node_id_for_node = |node_id| backend_node_ids.get(&node_id).copied();
            let document = self.vm().document_runtime.dom_host().dom();
            moli_dom::accessibility::accessibility_node_payload_for_document_with_backend_node_ids(
                document,
                document_handle,
                &mut backend_node_id_for_node,
            )
        };
        let payloads = payload.map(|payload| vec![payload]);
        Some(RendererAccessibilityPayloadsForObjectId {
            frame_id: Some(frame_id.to_owned()),
            payloads,
        })
    }

    fn accessibility_payloads_for_live_handle(
        &mut self,
        handle: DomHandle,
        build_payloads: impl FnOnce(
            &crate::dom::native::NativeDom,
            DomHandle,
            &HashMap<DomHandle, u32>,
        ) -> Option<Vec<serde_json::Value>>,
    ) -> Option<RendererAccessibilityPayloadsForObjectId> {
        let frame_id = self.vm().child_frame_id_for_live_node_handle(handle);
        let backend_node_ids = self.renderer_backend_node_id_map_for_owner_document(handle)?;
        let payloads = {
            let document = self.vm().document_runtime.dom_host().dom();
            build_payloads(document, handle, &backend_node_ids)?
        };
        Some(RendererAccessibilityPayloadsForObjectId {
            frame_id,
            payloads: Some(payloads),
        })
    }

    pub(crate) fn accessibility_tree_payloads_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
    ) -> Result<Option<RendererAccessibilityPayloadsForObjectId>> {
        self.accessibility_payloads_for_object_id(
            inspector_session_id,
            object_id,
            |document, node_id, backend_node_ids| {
                document.node(node_id)?;
                let mut backend_node_id_for_node =
                    |node_id| backend_node_ids.get(&node_id).copied();
                moli_dom::accessibility::accessibility_tree_payloads_for_document_with_backend_node_ids(
                    document,
                    node_id,
                    None,
                    &mut backend_node_id_for_node,
                )
            },
        )
    }

    pub(crate) fn accessibility_node_and_ancestor_payloads_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
    ) -> Result<Option<RendererAccessibilityPayloadsForObjectId>> {
        self.accessibility_payloads_for_object_id(
            inspector_session_id,
            object_id,
            |document, node_id, backend_node_ids| {
                document.node(node_id)?;
                let mut backend_node_id_for_node =
                    |node_id| backend_node_ids.get(&node_id).copied();
                moli_dom::accessibility::accessibility_node_and_ancestor_payloads_for_document_with_backend_node_ids(
                    document,
                    node_id,
                    &mut backend_node_id_for_node,
                )
            },
        )
    }

    pub(crate) fn accessibility_partial_tree_payloads_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
        fetch_relatives: bool,
    ) -> Result<Option<RendererAccessibilityPayloadsForObjectId>> {
        self.accessibility_payloads_for_object_id(
            inspector_session_id,
            object_id,
            |document, node_id, backend_node_ids| {
                let mut backend_node_id_for_node =
                    |node_id| backend_node_ids.get(&node_id).copied();
                moli_dom::accessibility::accessibility_partial_tree_payloads_for_document_with_backend_node_ids(
                    document,
                    node_id,
                    fetch_relatives,
                    &mut backend_node_id_for_node,
                )
            },
        )
    }

    fn accessibility_payloads_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
        build_payloads: impl FnOnce(
            &crate::dom::native::NativeDom,
            DomHandle,
            &HashMap<DomHandle, u32>,
        ) -> Option<Vec<serde_json::Value>>,
    ) -> Result<Option<RendererAccessibilityPayloadsForObjectId>> {
        let Some(handle) = self
            .vm_mut()
            .live_node_handle_for_runtime_object_id(inspector_session_id, object_id)?
        else {
            return Ok(None);
        };
        Ok(self.accessibility_payloads_for_live_handle(handle, build_payloads))
    }

    pub(crate) fn outer_html_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
        include_shadow_dom: bool,
    ) -> Result<Option<String>> {
        let Some(handle) = self
            .vm_mut()
            .live_node_handle_for_runtime_object_id(inspector_session_id, object_id)?
        else {
            return Ok(None);
        };
        Ok(self.outer_html_for_live_handle(handle, include_shadow_dom))
    }

    pub(crate) fn outer_html_for_backend_node_id(
        &mut self,
        backend_node_id: u32,
        include_shadow_dom: bool,
    ) -> Result<Option<String>> {
        let Some(handle) = self.live_handle_for_backend_node_id(backend_node_id) else {
            return Ok(None);
        };
        Ok(self.outer_html_for_live_handle(handle, include_shadow_dom))
    }

    pub(crate) fn outer_html_for_document(&self, include_shadow_dom: bool) -> Option<String> {
        let handle = self.vm().document_runtime.dom_host().document_handle();
        self.outer_html_for_live_handle(handle, include_shadow_dom)
    }

    fn outer_html_for_live_handle(
        &self,
        handle: DomHandle,
        include_shadow_dom: bool,
    ) -> Option<String> {
        self.vm()
            .outer_html_for_live_node_handle(handle, include_shadow_dom)
    }

    pub(crate) fn serialize_html(&self) -> String {
        self.vm()
            .document_runtime
            .dom_host()
            .dom()
            .serialize_document()
    }

    pub(crate) fn client_rect_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
    ) -> Result<Option<RendererDocumentNodeClientRect>> {
        let Some(handle) = self
            .vm_mut()
            .live_node_handle_for_runtime_object_id(inspector_session_id, object_id)?
        else {
            return Ok(None);
        };
        self.client_rect_for_live_node_handle(handle)
    }

    pub(crate) fn client_rect_for_backend_node_id(
        &mut self,
        backend_node_id: u32,
    ) -> Result<Option<RendererDocumentNodeClientRect>> {
        let Some(handle) = self.live_handle_for_backend_node_id(backend_node_id) else {
            return Ok(None);
        };
        self.client_rect_for_live_node_handle(handle)
    }

    pub(crate) fn document_geometry_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
    ) -> Result<Option<RendererDocumentNodeGeometry>> {
        let Some(handle) = self
            .vm_mut()
            .live_node_handle_for_runtime_object_id(inspector_session_id, object_id)?
        else {
            return Ok(None);
        };
        self.document_geometry_for_live_node_handle(handle)
            .map(Some)
    }

    pub(crate) fn document_geometry_for_backend_node_id(
        &mut self,
        backend_node_id: u32,
    ) -> Result<Option<RendererDocumentNodeGeometry>> {
        let Some(handle) = self.live_handle_for_backend_node_id(backend_node_id) else {
            return Ok(None);
        };
        self.document_geometry_for_live_node_handle(handle)
            .map(Some)
    }

    fn document_geometry_for_live_node_handle(
        &mut self,
        handle: DomHandle,
    ) -> Result<RendererDocumentNodeGeometry> {
        let (is_element, is_text, document) = {
            let host = self.vm().document_runtime.dom_host();
            let Some(node) = host.node(handle) else {
                return Ok(RendererDocumentNodeGeometry::NoLayoutObject);
            };
            let Some(document) = host.owner_document_handle(handle) else {
                return Ok(RendererDocumentNodeGeometry::NoLayoutObject);
            };
            (node.is_element(), node.is_text(), document)
        };
        if !is_element && !is_text {
            return Ok(RendererDocumentNodeGeometry::NotElement);
        }

        let answers = self
            .vm_mut()
            .observable_geometry_batch_for_document(
                document,
                moli_layout::LayoutFlushReason::CdpGeometry,
                &moli_layout::LayoutQueryBatch::new(vec![
                    moli_layout::LayoutQuery::BoxModel { source: handle },
                    moli_layout::LayoutQuery::ContentQuads { source: handle },
                    moli_layout::LayoutQuery::ElementMetrics { source: handle },
                ]),
            )
            .map_err(|error| anyhow::anyhow!("failed to resolve document geometry: {error}"))?;
        let mut answers = answers.answers.into_iter();
        let Some(moli_layout::LayoutQueryAnswer::BoxModel(box_model)) = answers.next() else {
            return Err(anyhow::anyhow!(
                "geometry provider returned a mismatched box-model answer"
            ));
        };
        let Some(moli_layout::LayoutQueryAnswer::ContentQuads(content_quads)) = answers.next()
        else {
            return Err(anyhow::anyhow!(
                "geometry provider returned a mismatched content-quads answer"
            ));
        };
        let Some(moli_layout::LayoutQueryAnswer::ElementMetrics(element_metrics)) = answers.next()
        else {
            return Err(anyhow::anyhow!(
                "geometry provider returned a mismatched element-metrics answer"
            ));
        };
        let mut composed_quads = Vec::new();
        if let Some(model) = box_model {
            composed_quads.extend([model.content, model.padding, model.border, model.margin]);
        }
        let model_quad_count = composed_quads.len();
        composed_quads.extend(content_quads);
        self.vm_mut()
            .compose_layout_quads_to_top(document, &mut composed_quads)
            .map_err(|error| anyhow::anyhow!("failed to compose frame geometry: {error}"))?;
        let content_quads = composed_quads[model_quad_count..]
            .iter()
            .copied()
            .map(renderer_geometry_quad)
            .collect();

        if !is_element {
            return Ok(RendererDocumentNodeGeometry::FoundNonElement { content_quads });
        }
        let (Some(_), Some(element_metrics)) = (box_model, element_metrics) else {
            return Ok(RendererDocumentNodeGeometry::NoLayoutObject);
        };
        let [content, padding, border, margin] = composed_quads[..model_quad_count] else {
            return Err(anyhow::anyhow!(
                "geometry provider returned an incomplete box model"
            ));
        };
        Ok(RendererDocumentNodeGeometry::FoundElement {
            box_model: Box::new(RendererDocumentBoxModel {
                content: renderer_geometry_quad(content),
                padding: renderer_geometry_quad(padding),
                border: renderer_geometry_quad(border),
                margin: renderer_geometry_quad(margin),
                width: rounded_css_integer(element_metrics.offset_size.width),
                height: rounded_css_integer(element_metrics.offset_size.height),
            }),
            content_quads,
        })
    }

    pub(crate) fn document_hit_test(
        &mut self,
        inspector_session_id: Option<&str>,
        x: f64,
        y: f64,
        include_user_agent_shadow_dom: bool,
        ignore_pointer_events_none: bool,
    ) -> Result<Option<RendererDocumentHitTestResult>> {
        // Generated UA shadow nodes are inspector projections rather than
        // layout sources, so their layout hit is already the live host.
        let _ = include_user_agent_shadow_dom;
        let hit = self
            .vm_mut()
            .observable_deep_hit_test_for_current_document(
                moli_layout::LayoutPoint::new(x as f32, y as f32),
                ignore_pointer_events_none,
            )
            .map_err(|error| anyhow::anyhow!("failed to hit test document: {error}"))?;
        let Some(mut handle) = hit else {
            return Ok(None);
        };
        loop {
            let Some(node) = self.vm().document_runtime.dom_host().node(handle) else {
                return Ok(None);
            };
            if node.is_element() {
                break;
            }
            let Some(parent) = node.parent_node() else {
                return Ok(None);
            };
            handle = parent;
        }
        let frame_id = self.vm().child_frame_id_for_live_node_handle(handle);
        let Some(node) = self.document_node_reference_for_live_handle(inspector_session_id, handle)
        else {
            return Ok(None);
        };
        Ok(Some(RendererDocumentHitTestResult { node, frame_id }))
    }

    fn client_rect_for_live_node_handle(
        &mut self,
        handle: DomHandle,
    ) -> Result<Option<RendererDocumentNodeClientRect>> {
        let Some((is_element, is_text)) = self
            .vm()
            .document_runtime
            .dom_host()
            .node(handle)
            .map(|node| (node.is_element(), node.is_text()))
        else {
            return Ok(None);
        };
        let rect = self
            .vm_mut()
            .client_rect_for_live_node_handle(handle)
            .map_err(|error| anyhow::anyhow!("failed to resolve client rect: {error}"))?;
        if is_element {
            Ok(rect.map(RendererDocumentNodeClientRect::Found))
        } else if is_text {
            Ok(rect.map(RendererDocumentNodeClientRect::FoundNonElement))
        } else {
            Ok(Some(RendererDocumentNodeClientRect::NotElement))
        }
    }

    pub(crate) fn node_has_geometry_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
    ) -> Result<Option<bool>> {
        let Some(handle) = self
            .vm_mut()
            .live_node_handle_for_runtime_object_id(inspector_session_id, object_id)?
        else {
            return Ok(None);
        };
        self.node_has_geometry_for_live_node_handle(handle)
    }

    pub(crate) fn node_has_geometry_for_backend_node_id(
        &mut self,
        backend_node_id: u32,
    ) -> Result<Option<bool>> {
        let Some(handle) = self.live_handle_for_backend_node_id(backend_node_id) else {
            return Ok(None);
        };
        self.node_has_geometry_for_live_node_handle(handle)
    }

    pub(crate) fn scroll_node_into_view_if_needed_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
        rect: Option<moli_page_types::DomScrollIntoViewRect>,
    ) -> Result<RendererScrollIntoViewResult> {
        let Some(handle) = self
            .vm_mut()
            .live_node_handle_for_runtime_object_id(inspector_session_id, object_id)?
        else {
            return Ok(RendererScrollIntoViewResult::NodeNotFound);
        };
        self.vm_mut()
            .scroll_live_node_handle_into_view_if_needed(handle, rect)
    }

    pub(crate) fn scroll_backend_node_into_view_if_needed(
        &mut self,
        backend_node_id: u32,
        rect: Option<moli_page_types::DomScrollIntoViewRect>,
    ) -> Result<RendererScrollIntoViewResult> {
        let Some(handle) = self.live_handle_for_backend_node_id(backend_node_id) else {
            return Ok(RendererScrollIntoViewResult::NodeNotFound);
        };
        self.vm_mut()
            .scroll_live_node_handle_into_view_if_needed(handle, rect)
    }

    fn node_has_geometry_for_live_node_handle(
        &mut self,
        handle: DomHandle,
    ) -> Result<Option<bool>> {
        let Some((is_document, is_geometry_node, document)) = self
            .vm()
            .document_runtime
            .dom_host()
            .node(handle)
            .and_then(|node| {
                Some((
                    node.is_document(),
                    node.is_element() || node.is_text(),
                    self.vm()
                        .document_runtime
                        .dom_host()
                        .owner_document_handle(handle)?,
                ))
            })
        else {
            return Ok(None);
        };
        if is_document {
            return Ok(Some(true));
        }
        if !is_geometry_node {
            return Ok(Some(false));
        }
        let answers = self
            .vm_mut()
            .observable_geometry_batch_for_document(
                document,
                moli_layout::LayoutFlushReason::CdpGeometry,
                &moli_layout::LayoutQueryBatch::new(vec![moli_layout::LayoutQuery::ClientRects {
                    source: handle,
                }]),
            )
            .map_err(|error| anyhow::anyhow!("failed to test node geometry: {error}"))?;
        let Some(moli_layout::LayoutQueryAnswer::ClientRects(rects)) =
            answers.answers.into_iter().next()
        else {
            return Err(anyhow::anyhow!(
                "geometry provider returned a mismatched client-rects answer"
            ));
        };
        Ok(Some(!rects.is_empty()))
    }

    pub(crate) fn document_frontend_node_ids_for_backend_node_ids(
        &mut self,
        inspector_session_id: Option<&str>,
        backend_node_ids: &[u32],
    ) -> RendererDocumentFrontendNodeIdsResolution {
        let include_whitespace = self.document_dom_agent_includes_whitespace(inspector_session_id);
        let document_handle = self
            .vm()
            .document_runtime
            .dom_host()
            .dom()
            .document_node_id();
        let Some(document_backend_node_id) =
            self.renderer_backend_node_id_for_live_handle(document_handle)
        else {
            return RendererDocumentFrontendNodeIdsResolution::DocumentNotBound;
        };
        if !self.document_has_frontend_node_id_for_backend_node_id(
            inspector_session_id,
            document_backend_node_id,
        ) {
            return RendererDocumentFrontendNodeIdsResolution::DocumentNotBound;
        }

        RendererDocumentFrontendNodeIdsResolution::Found(
            backend_node_ids
                .iter()
                .map(|backend_node_id| {
                    let handle = self.live_handle_for_backend_node_id(*backend_node_id)?;
                    let backend_node_id = self.renderer_backend_node_id_for_live_handle(handle)?;
                    let is_whitespace_text = inspector_whitespace_text_node(
                        self.vm().document_runtime.dom_host(),
                        handle,
                    );
                    Some(self
                        .document_frontend_node_id_for_backend_node_id_in_whitespace_projection(
                            inspector_session_id,
                            backend_node_id,
                            include_whitespace,
                            is_whitespace_text,
                        ))
                })
                .collect(),
        )
    }

    pub(crate) fn child_frame_owner_node_reference_by_frame_id(
        &mut self,
        inspector_session_id: Option<&str>,
        frame_id: &str,
    ) -> Option<RendererDocumentNodeReference> {
        let handle = self
            .vm()
            .child_browsing_context_owner_node_id_by_frame_id(frame_id)?;
        self.document_node_reference_for_live_handle(inspector_session_id, handle)
    }

    pub(crate) fn child_frame_document_root_node_reference_by_frame_id(
        &mut self,
        inspector_session_id: Option<&str>,
        frame_id: &str,
    ) -> Option<RendererDocumentNodeReference> {
        let handle = self
            .vm()
            .child_browsing_context_document_handle_by_frame_id(frame_id)?;
        self.document_node_reference_for_live_handle(inspector_session_id, handle)
    }

    fn document_node_reference_for_live_handle(
        &mut self,
        inspector_session_id: Option<&str>,
        handle: DomHandle,
    ) -> Option<RendererDocumentNodeReference> {
        let backend_node_id = self.renderer_backend_node_id_for_live_handle(handle)?;
        let node_id = self
            .document_frontend_node_id_for_backend_node_id(inspector_session_id, backend_node_id);
        Some(RendererDocumentNodeReference {
            node_id,
            backend_node_id,
        })
    }

    pub(super) fn live_handle_for_backend_node_id(
        &mut self,
        backend_node_id: u32,
    ) -> Option<DomHandle> {
        self.live_handle_for_renderer_backend_node_id(backend_node_id)
    }

    pub(crate) fn resolve_runtime_object_for_live_handle(
        &mut self,
        inspector_session_id: Option<&str>,
        handle: crate::dom::NodeId,
        execution_context_id: Option<i64>,
        object_group: Option<&str>,
    ) -> Result<Option<RendererRuntimeRemoteObject>> {
        const INTERNAL_RUNTIME_NODE_RESOLVE_ID: u64 = 900_002;
        let context_id = if let Some(context_id) =
            self.runtime_evaluate_context_id_for_resolution(execution_context_id)?
        {
            Some(context_id)
        } else if execution_context_id.is_some() {
            return Ok(None);
        } else {
            None
        };
        let token = self
            .vm_mut()
            .register_internal_node_reference(handle)
            .ok_or_else(|| anyhow!("live node handle is unavailable for runtime object resolve"))?;
        let result = (|| {
            let mut params = json!({
                "expression": format!(
                    "__moliHostResolveInternalNodeReference({token})"
                ),
                "objectGroup": object_group.unwrap_or(""),
            });
            if let Some(context_id) = context_id {
                params["contextId"] = json!(context_id);
            }
            let request = json!({
                "id": INTERNAL_RUNTIME_NODE_RESOLVE_ID,
                "method": "Runtime.evaluate",
                "params": params,
            });
            let raw_request = serde_json::to_string(&request)?;
            let messages = self
                .vm_mut()
                .dispatch_inspector_protocol_message_for_session(
                    inspector_session_id,
                    &raw_request,
                )?;
            Ok(Self::runtime_remote_object_from_inspector_messages(
                &messages,
                INTERNAL_RUNTIME_NODE_RESOLVE_ID,
            ))
        })();
        self.vm_mut().discard_internal_node_reference(token);
        result
    }

    pub(crate) fn resolve_runtime_object_for_backend_node_id(
        &mut self,
        inspector_session_id: Option<&str>,
        backend_node_id: u32,
        execution_context_id: Option<i64>,
        object_group: Option<&str>,
    ) -> Result<RendererRuntimeRemoteObjectResolution> {
        let Some(handle) = self.live_handle_for_backend_node_id(backend_node_id) else {
            return Ok(RendererRuntimeRemoteObjectResolution::MissingNode);
        };
        let remote_object = self.resolve_runtime_object_for_live_handle(
            inspector_session_id,
            handle,
            execution_context_id,
            object_group,
        )?;
        match remote_object {
            Some(remote_object) => Ok(RendererRuntimeRemoteObjectResolution::Found(remote_object)),
            None if execution_context_id.is_some() => {
                Ok(RendererRuntimeRemoteObjectResolution::MissingContext)
            }
            None => Ok(RendererRuntimeRemoteObjectResolution::MissingNode),
        }
    }

    fn runtime_evaluate_context_id_for_resolution(
        &self,
        execution_context_id: Option<i64>,
    ) -> Result<Option<i64>> {
        match execution_context_id {
            Some(context_id) => {
                if let Some(inspector_context_id) =
                    self.inspector_execution_context_id_for_isolated_context(context_id)
                {
                    Ok(Some(inspector_context_id))
                } else if self.has_isolated_execution_context_id(context_id) {
                    Ok(None)
                } else {
                    Ok(Some(context_id))
                }
            }
            None => Ok(None),
        }
    }

    fn runtime_remote_object_from_inspector_messages(
        messages: &[RendererRuntimeInspectorMessage],
        response_id: u64,
    ) -> Option<RendererRuntimeRemoteObject> {
        let response = runtime_inspector_response_message(messages, response_id)?;
        if response.get("error").is_some() || response["result"]["exceptionDetails"].is_object() {
            return None;
        }
        RendererRuntimeRemoteObject::from_protocol_value(response["result"]["result"].clone())
    }

    pub(crate) fn run_page_surface_override_script(&mut self, source: &str) -> Result<()> {
        if self.script_execution_disabled() {
            return Ok(());
        }
        self.vm_mut().exec_runtime_turn(source, None)
    }

    pub(crate) fn run_document_start_script_now_with_runtime_state(
        &mut self,
        inspector_session_id: Option<&str>,
        script: &DocumentStartScript,
    ) -> Result<Option<(i64, bool)>> {
        if self.script_execution_disabled() {
            return Ok(None);
        }

        let mut created_world_before_script = false;
        if let Some(world_name) = script.world_name.as_deref() {
            let runtime_bindings =
                self.inspector_session_runtime_bindings_for_world(inspector_session_id, world_name);
            if !runtime_bindings.is_empty() && !self.has_isolated_world_named(world_name) {
                let execution_context_id = self.create_isolated_world(world_name, false)?;
                created_world_before_script = true;
                for binding in runtime_bindings {
                    self.install_runtime_binding(
                        &binding.name,
                        Some(world_name),
                        Some(execution_context_id),
                    )?;
                }
            }
        }

        Ok(self.vm_mut().run_document_start_script_now(script)?.map(
            |(execution_context_id, created)| {
                (execution_context_id, created || created_world_before_script)
            },
        ))
    }

    pub(crate) fn add_document_start_script_runtime_activity(
        &mut self,
        inspector_session_id: Option<&str>,
        script: &DocumentStartScript,
        run_immediately: bool,
    ) -> Result<Option<(i64, bool)>> {
        self.document_start_scripts.push(script.clone());
        let scripts = self.document_start_scripts.clone();
        self.vm_mut().set_stored_document_start_scripts(&scripts);
        if !run_immediately {
            return Ok(None);
        }
        self.run_document_start_script_now_with_runtime_state(inspector_session_id, script)
    }

    pub(crate) fn run_document_start_script_in_execution_context(
        &mut self,
        execution_context_id: i64,
        script: &DocumentStartScript,
    ) -> Result<()> {
        if self.script_execution_disabled() {
            return Ok(());
        }
        self.vm_mut()
            .run_document_start_script_in_execution_context(execution_context_id, script)
    }

    #[cfg(test)]
    pub(crate) fn set_stored_document_start_scripts(&mut self, scripts: &[DocumentStartScript]) {
        self.document_start_scripts = scripts.to_vec();
        self.vm_mut().set_stored_document_start_scripts(scripts);
    }

    pub(crate) fn remove_document_start_script_by_registry_key(&mut self, registry_key: &str) {
        self.document_start_scripts
            .retain(|script| script.registry_key.as_deref() != Some(registry_key));
        let scripts = self.document_start_scripts.clone();
        self.vm_mut().set_stored_document_start_scripts(&scripts);
    }

    pub(crate) fn set_stored_runtime_bindings(
        &mut self,
        bindings: &[crate::protocol_types::RuntimeBindingRegistration],
    ) {
        self.runtime_bindings = bindings.to_vec();
        self.vm_mut().set_stored_runtime_bindings(bindings);
    }

    pub(crate) fn set_inspector_session_runtime_bindings(
        &mut self,
        inspector_session_id: Option<&str>,
        bindings: &[crate::protocol_types::RuntimeBindingRegistration],
    ) {
        self.vm_mut()
            .set_inspector_session_runtime_bindings(inspector_session_id, bindings);
        let session_key = DevToolsSessionKey::from_wire_session_id(
            inspector_session_id.filter(|session_id| !session_id.is_empty()),
        );
        let remove = {
            let configuration = self
                .runtime_inspector_protocol_configurations
                .entry(session_key.clone())
                .or_default();
            configuration.runtime_bindings = bindings.to_vec();
            !configuration.requires_restore()
        };
        if remove {
            self.runtime_inspector_protocol_configurations
                .remove(&session_key);
        }
    }

    pub(crate) fn inspector_session_runtime_bindings_for_world(
        &self,
        inspector_session_id: Option<&str>,
        world_name: &str,
    ) -> Vec<crate::protocol_types::RuntimeBindingRegistration> {
        self.vm()
            .inspector_session_runtime_bindings(inspector_session_id)
            .into_iter()
            .filter(|binding| binding.execution_context_name.as_deref() == Some(world_name))
            .collect()
    }

    pub(crate) fn set_runtime_binding_state(
        &mut self,
        inspector_session_id: Option<&str>,
        stored_runtime_bindings: &[crate::protocol_types::RuntimeBindingRegistration],
        session_runtime_bindings: &[crate::protocol_types::RuntimeBindingRegistration],
    ) {
        // Target attachment applies binding state before exposing the session
        // to frontend commands. Even an empty state must establish the
        // concrete renderer DevTools session/output capability: otherwise an
        // auxiliary session whose first command is a non-V8 IO agent has no
        // session host through which to publish its terminal response.
        self.vm_mut()
            .ensure_runtime_inspector_session(inspector_session_id);
        self.set_stored_runtime_bindings(stored_runtime_bindings);
        self.set_inspector_session_runtime_bindings(inspector_session_id, session_runtime_bindings);
    }

    pub(crate) fn detach_runtime_inspector_session(
        &mut self,
        inspector_session_id: Option<&str>,
    ) -> bool {
        let detached = self
            .vm_mut()
            .detach_runtime_inspector_session(inspector_session_id);
        let session_key = DevToolsSessionKey::from_wire_session_id(
            inspector_session_id.filter(|session_id| !session_id.is_empty()),
        );
        self.runtime_inspector_protocol_configurations
            .remove(&session_key);
        detached
    }

    pub(crate) fn install_runtime_binding(
        &mut self,
        name: &str,
        execution_context_name: Option<&str>,
        execution_context_id: Option<i64>,
    ) -> Result<()> {
        self.vm_mut()
            .install_runtime_binding(name, execution_context_name, execution_context_id)
    }

    pub(crate) fn remove_runtime_binding(&mut self, name: &str) -> Result<()> {
        self.vm_mut().remove_runtime_binding(name)
    }

    pub(crate) fn remove_default_runtime_binding(&mut self, name: &str) -> Result<()> {
        self.vm_mut().remove_default_runtime_binding(name)
    }

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

    pub(crate) fn remove_document_backend_node_id(&mut self, backend_node_id: u32) -> Result<bool> {
        let Some(handle) = self.live_handle_for_backend_node_id(backend_node_id) else {
            return Ok(false);
        };
        self.remove_document_node_id(handle)
    }

    pub(crate) fn mutate_document_backend_node_attribute(
        &mut self,
        backend_node_id: u32,
        mutation: RendererDomAttributeMutation,
    ) -> Result<RendererDomAttributeMutationOutcome> {
        let Some(handle) = self.live_handle_for_backend_node_id(backend_node_id) else {
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
            self.document_frontend_node_binding(inspector_session_id, frontend_node_id)
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

    pub(crate) fn focus_document_backend_node(
        &mut self,
        backend_node_id: u32,
    ) -> Result<RendererDomFocusOutcome> {
        let Some(handle) = self.live_handle_for_backend_node_id(backend_node_id) else {
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

    pub(crate) fn trigger_autofill(
        &mut self,
        request: RendererAutofillTriggerRequest,
    ) -> Result<RendererAutofillTriggerOutcome> {
        let expected_document = if let Some(frame_id) = request.frame_id.as_deref() {
            let Some(document) = self
                .vm()
                .child_browsing_context_document_handle_by_frame_id(frame_id)
            else {
                return Ok(RendererAutofillTriggerOutcome::FrameNotFound);
            };
            document
        } else {
            self.vm().document_runtime.dom_host().document_handle()
        };
        let Some(handle) = self.live_handle_for_backend_node_id(request.field_id) else {
            return Ok(RendererAutofillTriggerOutcome::FieldNotFound);
        };
        let belongs_to_expected_document = if request.frame_id.is_some() {
            child_frame_document_contains_live_handle(
                self.vm().document_runtime.dom_host(),
                expected_document,
                handle,
            )
        } else {
            handle == expected_document
                || self
                    .vm()
                    .document_runtime
                    .dom_host()
                    .owner_document_handle(handle)
                    == Some(expected_document)
        };
        if !belongs_to_expected_document {
            return Ok(RendererAutofillTriggerOutcome::FieldNotFound);
        }
        self.vm_mut().trigger_autofill(handle, request)
    }
}
fn renderer_geometry_quad(quad: moli_layout::LayoutQuad) -> RendererGeometryQuad {
    let [top_left, top_right, bottom_right, bottom_left] = quad.points;
    RendererGeometryQuad {
        points: [
            f64::from(top_left.x),
            f64::from(top_left.y),
            f64::from(top_right.x),
            f64::from(top_right.y),
            f64::from(bottom_right.x),
            f64::from(bottom_right.y),
            f64::from(bottom_left.x),
            f64::from(bottom_left.y),
        ],
    }
}

fn rounded_css_integer(value: f32) -> i32 {
    value.round().clamp(i32::MIN as f32, i32::MAX as f32) as i32
}

fn script_truthy_predicate_expression(expression: &str) -> String {
    format!("((value) => typeof value === 'function' ? value() : value)(\n(\n{expression}\n)\n)")
}

fn evaluation_payload_is_truthy(payload: &Value) -> bool {
    if payload.get("exception").is_some() {
        return false;
    }

    match payload.get("type").and_then(Value::as_str) {
        Some("undefined") => false,
        Some("boolean") => payload
            .get("value")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        Some("number") => payload
            .get("value")
            .and_then(Value::as_f64)
            .is_some_and(|number| number != 0.0),
        Some("string") => payload
            .get("value")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty()),
        Some("bigint") => payload
            .get("unserializableValue")
            .and_then(Value::as_str)
            .is_some_and(|value| value != "0"),
        Some("object") if payload.get("subtype").and_then(Value::as_str) == Some("null") => false,
        Some(_) => true,
        None => payload
            .get("value")
            .map_or(!payload.is_null(), |value| !value.is_null()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DOM_STABLE_COMPLETE_BASE_WINDOW, DOM_STABLE_COMPLETE_RUNTIME_WINDOW,
        DOM_STABLE_INTERACTIVE_WINDOW, NodeType, child_content_document_belongs_to_top_target,
        dom_stable_window_for_snapshot, inspector_whitespace_text_value,
        live_document_node_snapshot, live_inspector_document_node_snapshot,
        script_truthy_sleep_for,
    };
    use crate::dom::native::{DomHost, NativeDom};

    #[test]
    fn inspector_whitespace_classification_matches_chromium() {
        assert!(inspector_whitespace_text_value(
            "\t\n\r \u{1680}\u{2003}\u{2028}\u{3000}"
        ));
        assert!(inspector_whitespace_text_value(""));
        for visible_separator in ['\u{0085}', '\u{00a0}', '\u{2029}', '\u{202f}'] {
            assert!(
                !inspector_whitespace_text_value(&visible_separator.to_string()),
                "Chromium keeps U+{:04X} in the default DOM projection",
                visible_separator as u32
            );
        }
    }

    #[test]
    fn document_type_snapshot_preserves_native_name_case() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://doctype-snapshot.test/").expect("test URL"),
        ));
        let document = host.document_handle();
        let doctype = host.create_document_type("html", "", "");
        assert!(host.append_child(document, doctype));

        let snapshot = live_document_node_snapshot(&host, document, -1, None, true)
            .expect("document snapshot");
        let doctype_snapshot = snapshot
            .children
            .iter()
            .find(|node| node.node_type == 10)
            .expect("doctype snapshot");

        assert_eq!(doctype_snapshot.node_name, "html");
        assert_eq!(doctype_snapshot.document_type_name.as_deref(), Some("html"));
    }

    #[test]
    fn inspector_depth_boundary_forces_an_only_text_child_like_chromium() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://single-text-child.test/").expect("test URL"),
        ));
        let title = host.create_element("title");
        let title_text = host.create_text_node("Example Domain");
        assert!(host.append_child(title, title_text));

        let generic_snapshot = live_document_node_snapshot(&host, title, 0, None, false)
            .expect("generic shallow snapshot");
        assert!(generic_snapshot.children.is_empty());

        let inspector_snapshot =
            live_inspector_document_node_snapshot(&host, title, 0, None, false, false)
                .expect("inspector shallow snapshot");
        assert_eq!(inspector_snapshot.child_count, 1);
        assert_eq!(inspector_snapshot.children.len(), 1);
        assert_eq!(
            inspector_snapshot.children[0].node_type,
            NodeType::Text as u8
        );
        assert_eq!(inspector_snapshot.children[0].node_value, "Example Domain");

        let second_text = host.create_text_node("second");
        assert!(host.append_child(title, second_text));
        let multiple_children =
            live_inspector_document_node_snapshot(&host, title, 0, None, false, false)
                .expect("multiple-child inspector snapshot");
        assert_eq!(multiple_children.child_count, 2);
        assert!(multiple_children.children.is_empty());
    }

    #[test]
    fn template_snapshot_exposes_shallow_content_fragment() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://template-snapshot.test/").expect("test URL"),
        ));
        let document = host.document_handle();
        let template = host.create_element("template");
        let template_content = host
            .parser_template_contents_handle(template)
            .expect("template content fragment");
        let article = host.create_element("article");
        assert!(host.append_child(template_content, article));
        assert!(host.append_child(document, template));

        let snapshot = live_document_node_snapshot(&host, document, -1, None, false)
            .expect("document snapshot");
        let template_snapshot = snapshot
            .children
            .iter()
            .find(|node| node.node_id == template)
            .expect("template snapshot");
        let content_snapshot = template_snapshot
            .template_content()
            .expect("template content snapshot");

        assert_eq!(template_snapshot.child_count, 0);
        assert_eq!(content_snapshot.node_id, template_content);
        assert_eq!(content_snapshot.node_type, NodeType::DocumentFragment as u8);
        assert_eq!(content_snapshot.parent_id, None);
        assert_eq!(content_snapshot.child_count, 1);
        assert!(
            content_snapshot.children.is_empty(),
            "a template host only projects the shallow content fragment"
        );

        let direct_content_snapshot =
            live_document_node_snapshot(&host, template_content, -1, None, false)
                .expect("direct template content snapshot");
        assert_eq!(direct_content_snapshot.children.len(), 1);
        assert_eq!(direct_content_snapshot.children[0].node_id, article);
        assert_eq!(direct_content_snapshot.children[0].local_name, "article");
    }

    #[test]
    fn inspector_content_document_stays_on_the_top_target_for_same_site_frames() {
        let top = url::Url::parse("https://app.example.test:8443/page").expect("top URL");
        let child = url::Url::parse("https://cdn.example.test/frame").expect("child URL");

        assert!(child_content_document_belongs_to_top_target(
            &top, &child, false, false
        ));
    }

    #[test]
    fn inspector_content_document_omits_cross_site_and_opaque_frames() {
        let top = url::Url::parse("http://127.0.0.1:8000/page").expect("top URL");
        let cross_site =
            url::Url::parse("http://localhost:8000/frame").expect("cross-site child URL");
        let inherited = url::Url::parse("about:srcdoc").expect("srcdoc URL");

        assert!(!child_content_document_belongs_to_top_target(
            &top,
            &cross_site,
            false,
            false,
        ));
        assert!(!child_content_document_belongs_to_top_target(
            &top, &inherited, true, true,
        ));
        assert!(child_content_document_belongs_to_top_target(
            &top, &inherited, true, false,
        ));
    }

    #[test]
    fn script_truthy_sleep_for_uses_remaining_when_no_runtime_timeout_is_pending() {
        let sleep_for = script_truthy_sleep_for(None, std::time::Duration::from_millis(180));
        assert_eq!(sleep_for, std::time::Duration::from_millis(180));
    }

    #[test]
    fn script_truthy_sleep_for_respects_next_runtime_timeout() {
        let sleep_for = script_truthy_sleep_for(Some(40), std::time::Duration::from_millis(180));
        assert_eq!(sleep_for, std::time::Duration::from_millis(40));
    }

    #[test]
    fn dom_stable_complete_window_stays_short_without_runtime_backlog() {
        let window =
            dom_stable_window_for_snapshot("complete|https://example.test/", false, false).unwrap();
        assert_eq!(window, DOM_STABLE_COMPLETE_BASE_WINDOW);
    }

    #[test]
    fn dom_stable_complete_window_expands_after_runtime_backlog_is_seen() {
        let window =
            dom_stable_window_for_snapshot("complete|https://example.test/", true, false).unwrap();
        assert_eq!(window, DOM_STABLE_COMPLETE_RUNTIME_WINDOW);
    }

    #[test]
    fn dom_stable_interactive_window_is_always_extended() {
        let window =
            dom_stable_window_for_snapshot("interactive|https://example.test/", false, false)
                .unwrap();
        assert_eq!(window, DOM_STABLE_INTERACTIVE_WINDOW);
    }

    #[test]
    fn dom_stable_complete_window_expands_for_long_pending_timeout() {
        let window =
            dom_stable_window_for_snapshot("complete|https://example.test/", false, true).unwrap();
        assert_eq!(window, DOM_STABLE_COMPLETE_RUNTIME_WINDOW);
    }
}

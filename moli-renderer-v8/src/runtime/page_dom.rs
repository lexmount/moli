#[cfg(feature = "ace-protocol")]
mod ace_page_task;

use super::*;

use crate::document_runtime::DomHandle;
use crate::dom::native::{DomHost, Node, NodeData, NodeType};
use crate::runtime::page_surface::{
    RendererDocumentQuerySelectorNode, RendererDocumentQuerySelectorResolution,
    RendererRuntimeEvaluationResult,
};
use crate::runtime::page_vm::PageVmRuntimeCommandOutputScopeId;
use crate::script_vm::{
    PendingRuntimeEvaluateCall, RuntimeEvaluateCodeGenerationPolicy, RuntimeEvaluateOutcome,
    RuntimeEvaluateResultMode,
};
use moli_dom::accessibility::AccessibilityRequest;
use moli_page_types::{
    DocumentNodeAssociatedSnapshot, DocumentNodeAttributeSnapshot, DocumentNodeSnapshot,
    MAX_DOM_OUTPUT_TREE_DEPTH,
};
use moli_selector::QueryEngine;

#[cfg(feature = "devtools")]
mod devtools;
#[cfg(not(feature = "devtools"))]
mod disabled;
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

#[cfg(feature = "devtools")]
pub(crate) fn inspector_whitespace_text_snapshot(snapshot: &DocumentNodeSnapshot) -> bool {
    snapshot.node_type == NodeType::Text as u8
        && inspector_whitespace_text_value(&snapshot.node_value)
}

#[cfg(any(test, feature = "devtools"))]
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

const DOM_STABLE_COMPLETE_BASE_WINDOW: std::time::Duration = std::time::Duration::from_millis(350);
const DOM_STABLE_COMPLETE_RUNTIME_WINDOW: std::time::Duration =
    std::time::Duration::from_millis(1200);
const DOM_STABLE_INTERACTIVE_WINDOW: std::time::Duration = std::time::Duration::from_millis(1200);

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

#[cfg(any(test, feature = "devtools"))]
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
    let shadow_roots = if let Some(next_tree_depth) = next_tree_depth {
        dom_host
            .shadow_root_handle(node_id)
            .and_then(|shadow_root_id| {
                live_document_node_snapshot_with_budget(
                    dom_host,
                    shadow_root_id,
                    if pierce { next_depth } else { 0 },
                    Some(node_id),
                    pierce,
                    include_whitespace,
                    force_single_text_child_at_depth_boundary,
                    next_tree_depth,
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
        match scope.lifecycle_document {
            None => {
                scope.lifecycle_document = Some(document);
                Ok(())
            }
            Some(existing) => {
                ensure!(
                    existing == document,
                    "runtime command lifecycle observer cannot be rebound to another Document"
                );
                Ok(())
            }
        }
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

    pub(crate) fn runtime_realm_inventory(&mut self) -> Vec<RendererRuntimeRealmInfo> {
        self.vm_mut().runtime_realm_inventory()
    }

    pub(crate) fn live_child_default_runtime_realm_inventory(
        &mut self,
    ) -> Vec<RendererRuntimeRealmInfo> {
        self.vm_mut().live_child_default_runtime_realm_inventory()
    }

    pub(crate) fn create_isolated_world(
        &mut self,
        name: &str,
        grant_universal_access: bool,
    ) -> Result<i64> {
        self.vm_mut()
            .create_isolated_world(name, grant_universal_access)
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
            || self.vm().has_pending_bitmap_tasks()
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
        _inspector_session_id: Option<&str>,
        handles: Vec<DomHandle>,
    ) -> Vec<RendererDocumentQuerySelectorNode> {
        handles
            .into_iter()
            .filter_map(|handle| {
                let backend_node_id = self.renderer_backend_node_id_for_live_handle(handle)?;
                #[cfg(feature = "devtools")]
                let frontend_node_id = self.document_frontend_node_id_for_backend_node_id(
                    _inspector_session_id,
                    backend_node_id,
                );
                #[cfg(not(feature = "devtools"))]
                let frontend_node_id = 0; // No frontend binding; the backend ID remains native.
                Some(RendererDocumentQuerySelectorNode {
                    frontend_node_id,
                    backend_node_id,
                })
            })
            .collect()
    }

    pub(crate) fn live_handle_for_document_node_reference(
        &mut self,
        reference: RendererDomNodeReference,
    ) -> Option<DomHandle> {
        let backend_node_id = self.backend_node_id_for_document_node_reference(reference)?;
        self.live_handle_for_backend_node_id(backend_node_id)
    }

    pub(crate) fn accessibility_tree_nodes(
        &self,
        frame_id: Option<&str>,
    ) -> Option<Vec<moli_dom::accessibility::AccessibilityNode>> {
        let document = match frame_id {
            Some(frame_id) => self
                .vm()
                .child_browsing_context_document_handle_by_frame_id(frame_id)?,
            None => self.vm().document_runtime.dom_host().document_node_id(),
        };
        self.vm().accessibility_nodes(
            document,
            AccessibilityRequest::Tree { max_depth: None },
            &mut |node_id| self.renderer_backend_node_id_for_live_handle(node_id),
        )
    }

    pub(crate) fn outer_html_for_node(
        &mut self,
        reference: RendererDomNodeReference,
        include_shadow_dom: bool,
    ) -> Result<Option<String>> {
        let Some(handle) = self.live_handle_for_document_node_reference(reference) else {
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

    pub(super) fn live_handle_for_backend_node_id(
        &mut self,
        backend_node_id: u32,
    ) -> Option<DomHandle> {
        self.live_handle_for_renderer_backend_node_id(backend_node_id)
    }

    pub(crate) fn run_page_surface_override_script(&mut self, source: &str) -> Result<()> {
        if self.script_execution_disabled() {
            return Ok(());
        }
        self.vm_mut().exec_runtime_turn(source, None)
    }

    #[cfg(test)]
    pub(crate) fn set_stored_document_start_scripts(&mut self, scripts: &[DocumentStartScript]) {
        self.document_start_scripts = scripts.to_vec();
        self.vm_mut().set_stored_document_start_scripts(scripts);
    }
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

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap},
    pin::pin,
    rc::Rc,
    time::Instant,
};

use crate::{
    DocumentStartScript,
    content_security_policy::ContentSecurityPolicyScriptElementRequest,
    dom::{
        NodeId,
        native::{Attribute, DomHost, DomMutationEffects, NativeDom, NativeNodeId, NodeData},
    },
    frame_owner_model::{
        ChildDocumentModulatorStore, DocumentId, FrameDocumentTaskOwner, FrameOwnerStore,
        FrameRealmId, FrameScriptJob, FrameScriptJobKind,
    },
    inspector_microtasks::with_scoped_inspector_microtasks,
    inspector_session::dispatch_with_runtime_defaults,
    network::{ResourceRequestClient, context::DocumentResourceLoader},
    page_task_queue::{
        PageRuntimeWakeSender, PageTask, PageTaskSender, PostParseLifecycleWork,
        RendererResourceCompletionSender, RuntimePageTaskSender,
    },
    runtime::{
        RendererBrowserContextRuntime, RendererCountEntry, RendererMoliDomMemoryDiagnostics,
        RendererMoliMemoryDiagnostics, RendererMoliMemoryScopeDiagnostics,
        RendererMoliRuntimeMemoryDiagnostics, RendererPerformanceMetricSnapshot,
        RendererRuntimeHeapSpaceUsage, RendererRuntimeHeapUsage,
        RendererRuntimeInspectorAsyncCompletion, RendererRuntimeInspectorMessage,
        RendererRuntimeInspectorResponseSender, RendererRuntimeRealmInfo,
        RendererScriptExecutionMemoryDiagnostics, RendererScriptSourceMemoryDiagnostics,
        RendererScrollIntoViewResult, RuntimeConsoleMessageSnapshot,
        SharedRendererBackendNodeRegistry,
    },
    runtime_binding_data::{build_runtime_binding_data, runtime_binding_callback},
    script_provenance::CompiledStringProvenance,
    types::ScriptObservableOutput,
};
use anyhow::{Context, Result, anyhow};
use moli_page_types::{
    DevToolsSessionKey, RendererDomDebuggerDomBreakpointType,
    RendererDomDebuggerEventListenerBreakpoint, RendererDomDebuggerXhrBreakpoint,
    RendererInspectorProtocolConfiguration, V8InspectorSessionAttach,
};

#[cfg(any(test, feature = "test-support"))]
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::debug;
use url::Url;

pub(crate) type ScriptVmBootstrapError = Box<(anyhow::Error, DomHost)>;

fn renderer_document_isolate_critical_pressure_required(
    used_heap_size: usize,
    heap_size_limit: usize,
) -> bool {
    if heap_size_limit == 0 {
        return false;
    }
    let critical_threshold = heap_size_limit / 3 + usize::from(!heap_size_limit.is_multiple_of(3));
    used_heap_size >= critical_threshold
}

#[cfg(test)]
fn expect_ready_child_frame_owner_source_future_for_test<F>(future: F) -> F::Output
where
    F: std::future::Future,
{
    use std::task::{Context as TaskContext, Poll, Waker};

    let mut future = pin!(future);
    let waker = Waker::noop();
    let mut cx = TaskContext::from_waker(waker);
    match future.as_mut().poll(&mut cx) {
        Poll::Ready(output) => output,
        Poll::Pending => {
            panic!("child frame owner-source future returned Pending in a synchronous test turn");
        }
    }
}

const PERFORMANCE_METRICS_SNAPSHOT_EXPRESSION: &str = r#"
(() => {
  const numeric = (value) => {
    const number = Number(value);
    return Number.isFinite(number) ? number : 0;
  };
  const perf = globalThis.performance || {};
  const timing = perf.timing || {};
  const timeOriginMs =
    numeric(perf.timeOrigin) || numeric(timing.navigationStart) || Date.now();
  const nowMs = typeof perf.now === "function" ? numeric(perf.now()) : 0;
  const navigationStartMs = numeric(timing.navigationStart) || timeOriginMs;
  const domContentLoadedMs =
    numeric(timing.domContentLoadedEventEnd) ||
    numeric(timing.domContentLoadedEventStart) ||
    timeOriginMs + nowMs;
  const loadEventMs =
    numeric(timing.loadEventEnd) ||
    numeric(timing.loadEventStart) ||
    domContentLoadedMs;

  let nodeCount = 0;
  let documentCount = 0;
  let frameCount = 0;
  let resourceCount = 0;
  try {
    if (globalThis.document) {
      documentCount = 1;
      nodeCount = 1;
      if (document.querySelectorAll) {
        nodeCount += document.querySelectorAll("*").length;
        frameCount = document.querySelectorAll("iframe,frame").length;
      }
      frameCount += 1;
    }
  } catch (_error) {
  }
  try {
    if (typeof perf.getEntriesByType === "function") {
      resourceCount = perf.getEntriesByType("resource").length;
    } else if (typeof perf.getEntries === "function") {
      resourceCount = perf.getEntries().length;
    }
  } catch (_error) {
  }

  return JSON.stringify({
    timeOriginMs,
    nowMs,
    navigationStartMs,
    domContentLoadedMs,
    loadEventMs,
    documentCount,
    frameCount,
    nodeCount,
    resourceCount,
  });
})()
"#;

fn moli_dom_memory_counters(document: &NativeDom) -> RendererMoliDomMemoryDiagnostics {
    let mut node_counts = BTreeMap::<String, usize>::new();
    let mut element_tags = BTreeMap::<String, usize>::new();
    let mut connected_nodes = 0usize;
    let mut in_document_tree_nodes = 0usize;
    let mut parser_created_nodes = 0usize;
    let mut attribute_count = 0usize;
    let mut attribute_name_bytes = 0usize;
    let mut attribute_value_bytes = 0usize;
    let mut element_name_bytes = 0usize;
    let mut text_node_count = 0usize;
    let mut text_bytes = 0usize;
    let mut inline_script_text_bytes = 0usize;
    let mut comment_bytes = 0usize;
    let mut cdata_bytes = 0usize;
    let mut processing_instruction_bytes = 0usize;
    let mut script_element_count = 0usize;
    let mut external_script_count = 0usize;
    let mut external_script_src_bytes = 0usize;
    let mut image_element_count = 0usize;
    let mut iframe_element_count = 0usize;
    let mut style_element_count = 0usize;
    let mut link_stylesheet_count = 0usize;
    let mut template_content_count = 0usize;

    for node in document.nodes() {
        *node_counts.entry(node.kind_name().to_owned()).or_default() += 1;
        if node.is_connected() {
            connected_nodes += 1;
        }
        if node.flags().in_document_tree() {
            in_document_tree_nodes += 1;
        }
        if node.flags().parser_created() {
            parser_created_nodes += 1;
        }

        match node.data() {
            NodeData::Element(element) => {
                *element_tags
                    .entry(element.local_name().to_owned())
                    .or_default() += 1;
                element_name_bytes += element.local_name().len()
                    + element.namespace().len()
                    + element.prefix().map(str::len).unwrap_or_default();
                attribute_count += element.attributes().len();
                for attribute in element.attributes() {
                    attribute_name_bytes += attribute.local_name().len()
                        + attribute.namespace().len()
                        + attribute.prefix().map(str::len).unwrap_or_default();
                    attribute_value_bytes += attribute.value().len();
                }
                if element.is_html_element("script") {
                    script_element_count += 1;
                    if let Some(src) = element.attribute("src") {
                        external_script_count += 1;
                        external_script_src_bytes += src.len();
                    }
                } else if element.is_html_element("img") {
                    image_element_count += 1;
                } else if element.is_html_element("iframe") || element.is_html_element("frame") {
                    iframe_element_count += 1;
                } else if element.is_inline_style_element() {
                    style_element_count += 1;
                } else if element.is_html_element("link")
                    && element
                        .attribute("rel")
                        .is_some_and(|rel| rel.eq_ignore_ascii_case("stylesheet"))
                {
                    link_stylesheet_count += 1;
                }
                if element.template_contents().is_some() {
                    template_content_count += 1;
                }
            }
            NodeData::Text(text) => {
                text_node_count += 1;
                let len = text.data().len();
                text_bytes += len;
                if parent_is_html_element(document, node.parent_node(), "script") {
                    inline_script_text_bytes += len;
                }
            }
            NodeData::CDataSection(cdata) => {
                cdata_bytes += cdata.data().len();
            }
            NodeData::Comment(comment) => {
                comment_bytes += comment.data().len();
            }
            NodeData::ProcessingInstruction(processing_instruction) => {
                processing_instruction_bytes +=
                    processing_instruction.target().len() + processing_instruction.data().len();
            }
            NodeData::Document(_) | NodeData::DocumentType(_) | NodeData::DocumentFragment(_) => {}
        }
    }

    let string_payload_bytes = attribute_name_bytes
        + attribute_value_bytes
        + element_name_bytes
        + text_bytes
        + comment_bytes
        + cdata_bytes
        + processing_instruction_bytes;

    RendererMoliDomMemoryDiagnostics {
        node_count: document.len(),
        connected_node_count: connected_nodes,
        in_document_tree_node_count: in_document_tree_nodes,
        parser_created_node_count: parser_created_nodes,
        node_counts_by_kind: node_counts,
        top_element_tags: top_count_entries(element_tags, 16),
        attribute_count,
        attribute_name_bytes,
        attribute_value_bytes,
        element_name_bytes,
        text_node_count,
        text_bytes,
        comment_bytes,
        cdata_bytes,
        processing_instruction_bytes,
        string_payload_bytes,
        script_element_count,
        external_script_count,
        external_script_src_bytes,
        inline_script_text_bytes,
        image_element_count,
        iframe_element_count,
        style_element_count,
        link_stylesheet_count,
        template_content_count,
        parse_error_count: document.parse_errors().len(),
    }
}

fn runtime_protocol_message_user_gesture(raw_json: &str) -> bool {
    let Ok(message) = serde_json::from_str::<Value>(raw_json) else {
        return false;
    };
    match message.get("method").and_then(Value::as_str) {
        Some("Runtime.evaluate") | Some("Runtime.callFunctionOn") => message
            .get("params")
            .and_then(|params| params.get("userGesture"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        _ => false,
    }
}

#[derive(Clone, Copy)]
enum InspectorWindowDispatchTarget {
    DefaultTop,
    ExecutionContext(i64),
}

fn runtime_protocol_message_window_dispatch_target(
    raw_json: &str,
) -> Option<InspectorWindowDispatchTarget> {
    let message = serde_json::from_str::<Value>(raw_json).ok()?;
    let params = message.get("params")?;
    match message.get("method").and_then(Value::as_str) {
        Some("Runtime.evaluate") | Some("Runtime.compileScript") => Some(
            params
                .get("contextId")
                .or_else(|| params.get("executionContextId"))
                .and_then(Value::as_i64)
                .map(InspectorWindowDispatchTarget::ExecutionContext)
                .unwrap_or(InspectorWindowDispatchTarget::DefaultTop),
        ),
        Some("Runtime.callFunctionOn") | Some("Runtime.runScript") => params
            .get("executionContextId")
            .and_then(Value::as_i64)
            .map(InspectorWindowDispatchTarget::ExecutionContext),
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct InspectorWindowDispatchScope {
    context_ptr: *const v8::Global<v8::Context>,
    child_handle: Option<DomHandle>,
}

fn enter_inspector_window_dispatch_scope(
    scope: &mut v8::PinScope<'_, '_>,
    owner: InspectorWindowDispatchScope,
) -> v8::Global<v8::Value> {
    let context = unsafe { v8::Local::new(scope, &*owner.context_ptr) };
    let owner_scope = &mut v8::ContextScope::new(scope, context);
    let previous =
        crate::native_bridge::enter_active_child_window_scope(owner_scope, owner.child_handle);
    v8::Global::new(owner_scope, previous)
}

fn restore_inspector_window_dispatch_scope(
    scope: &mut v8::PinScope<'_, '_>,
    owner: InspectorWindowDispatchScope,
    previous: &v8::Global<v8::Value>,
) {
    let context = unsafe { v8::Local::new(scope, &*owner.context_ptr) };
    let owner_scope = &mut v8::ContextScope::new(scope, context);
    let previous = v8::Local::new(owner_scope, previous);
    crate::native_bridge::restore_active_child_window_scope(owner_scope, previous);
}

fn runtime_protocol_message_runs_embedder_microtask_checkpoint(raw_json: &str) -> bool {
    let Ok(message) = serde_json::from_str::<Value>(raw_json) else {
        return true;
    };
    !message
        .get("method")
        .and_then(Value::as_str)
        .is_some_and(|method| method.starts_with("Debugger."))
}

pub(crate) const WEBDRIVER_BIDI_FILE_PROMPT_HANDLER_PARAM: &str =
    "__moliWebDriverBidiFilePromptHandler";
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RuntimeEvaluateCodeGenerationPolicy {
    AllowDuringEvaluation,
    EnforceContextPolicy,
}

impl RuntimeEvaluateCodeGenerationPolicy {
    pub(super) fn from_cdp(value: Option<bool>) -> Self {
        if value.unwrap_or(true) {
            Self::AllowDuringEvaluation
        } else {
            Self::EnforceContextPolicy
        }
    }

    fn allows_unsafe_eval_blocked_by_csp(self) -> bool {
        matches!(self, Self::AllowDuringEvaluation)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RuntimeEvaluateResultMode {
    /// Preserve the ordinary `Runtime.evaluate` representation, including a
    /// remote object handle when the result cannot be returned as a primitive.
    RemoteObject,
    /// Serialize the result into a JSON-compatible protocol value instead of
    /// returning a remote object handle.
    ByValue,
}

impl RuntimeEvaluateResultMode {
    const fn returns_by_value(self) -> bool {
        matches!(self, Self::ByValue)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PendingRuntimeEvaluateCall {
    call_id: i32,
}

pub(super) enum RuntimeEvaluateOutcome {
    Complete(Value),
    Pending(PendingRuntimeEvaluateCall),
}

fn runtime_protocol_message_file_prompt_handler(raw_json: &str) -> Option<String> {
    let Ok(message) = serde_json::from_str::<Value>(raw_json) else {
        return None;
    };
    match message.get("method").and_then(Value::as_str) {
        Some("Runtime.evaluate") | Some("Runtime.callFunctionOn") => message
            .get("params")
            .and_then(|params| params.get(WEBDRIVER_BIDI_FILE_PROMPT_HANDLER_PARAM))
            .and_then(Value::as_str)
            .filter(|handler| matches!(*handler, "accept" | "dismiss"))
            .map(str::to_owned),
        _ => None,
    }
}

fn runtime_binding_replay_request_json(
    binding: &crate::protocol_types::RuntimeBindingRegistration,
    index: usize,
) -> Result<(i32, String)> {
    let replay_id = i64::from(900_100_000_i32)
        .saturating_add(i64::try_from(index).unwrap_or(i64::from(i32::MAX)))
        .min(i64::from(i32::MAX));
    let replay_call_id = i32::try_from(replay_id).expect("bounded inspector replay call id");
    let mut params = serde_json::Map::new();
    params.insert("name".to_owned(), json!(binding.name));
    if let Some(execution_context_name) = &binding.execution_context_name {
        params.insert(
            "executionContextName".to_owned(),
            json!(execution_context_name),
        );
    }
    let request = serde_json::to_string(&json!({
        "id": replay_id,
        "method": "Runtime.addBinding",
        "params": Value::Object(params),
    }))
    .context("runtime binding replay request should serialize")?;
    Ok((replay_call_id, request))
}

struct RuntimeBindingReplayGlobalSnapshot<'s> {
    context: v8::Local<'s, v8::Context>,
    key: v8::Local<'s, v8::String>,
    value: v8::Local<'s, v8::Value>,
}

fn capture_runtime_binding_replay_global_snapshots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context_ptrs: &[*const v8::Global<v8::Context>],
    bindings: &[crate::protocol_types::RuntimeBindingRegistration],
) -> Vec<RuntimeBindingReplayGlobalSnapshot<'s>> {
    let mut snapshots = Vec::new();
    for &context_ptr in context_ptrs {
        let context = unsafe { v8::Local::new(scope, &*context_ptr) };
        let scope = &mut v8::ContextScope::new(scope, context);
        let global = context.global(scope);
        for binding in bindings {
            let Some(key) = v8_string(scope, &binding.name) else {
                continue;
            };
            if !global.has_own_property(scope, key.into()).unwrap_or(false) {
                continue;
            }
            let Some(value) = global.get(scope, key.into()) else {
                continue;
            };
            snapshots.push(RuntimeBindingReplayGlobalSnapshot {
                context,
                key,
                value,
            });
        }
    }
    snapshots
}

fn restore_runtime_binding_replay_global_snapshots(
    scope: &mut v8::PinScope<'_, '_>,
    snapshots: Vec<RuntimeBindingReplayGlobalSnapshot<'_>>,
) {
    for snapshot in snapshots {
        let scope = &mut v8::ContextScope::new(scope, snapshot.context);
        let global = snapshot.context.global(scope);
        let _ = global.set(scope, snapshot.key.into(), snapshot.value);
    }
}

fn parent_is_html_element(document: &NativeDom, parent: Option<NodeId>, local_name: &str) -> bool {
    parent
        .and_then(|parent| document.node(parent))
        .and_then(|parent| parent.as_element())
        .is_some_and(|element| element.is_html_element(local_name))
}

fn top_count_entries(counts: BTreeMap<String, usize>, limit: usize) -> Vec<RendererCountEntry> {
    let mut entries = counts
        .into_iter()
        .map(|(name, count)| RendererCountEntry { name, count })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.name.cmp(&right.name))
    });
    entries.truncate(limit);
    entries
}

#[derive(Default)]
struct ScriptExecutionMemoryCounters {
    execution_count: usize,
    total_source_bytes: usize,
    inline_source_bytes: usize,
    external_source_bytes: usize,
    classic_source_bytes: usize,
    module_source_bytes: usize,
    import_map_source_bytes: usize,
    data_block_source_bytes: usize,
    inline_execution_count: usize,
    external_execution_count: usize,
    classic_execution_count: usize,
    module_execution_count: usize,
    import_map_execution_count: usize,
    data_block_execution_count: usize,
    largest_sources: Vec<ScriptSourceMemorySample>,
}

#[derive(Clone)]
struct ScriptSourceMemorySample {
    url: String,
    source_bytes: usize,
    kind: ScriptKind,
    mode: ScriptMode,
    source_kind: ScriptSourceKind,
}

impl ScriptExecutionMemoryCounters {
    fn record(&mut self, script: &PreparedScript, source_len: usize) {
        self.execution_count += 1;
        self.total_source_bytes = self.total_source_bytes.saturating_add(source_len);
        match script.source_kind {
            ScriptSourceKind::Inline => {
                self.inline_execution_count += 1;
                self.inline_source_bytes = self.inline_source_bytes.saturating_add(source_len);
            }
            ScriptSourceKind::External => {
                self.external_execution_count += 1;
                self.external_source_bytes = self.external_source_bytes.saturating_add(source_len);
            }
        }
        match script.kind {
            ScriptKind::Classic => {
                self.classic_execution_count += 1;
                self.classic_source_bytes = self.classic_source_bytes.saturating_add(source_len);
            }
            ScriptKind::Module => {
                self.module_execution_count += 1;
                self.module_source_bytes = self.module_source_bytes.saturating_add(source_len);
            }
            ScriptKind::ImportMap => {
                self.import_map_execution_count += 1;
                self.import_map_source_bytes =
                    self.import_map_source_bytes.saturating_add(source_len);
            }
            ScriptKind::DataBlock => {
                self.data_block_execution_count += 1;
                self.data_block_source_bytes =
                    self.data_block_source_bytes.saturating_add(source_len);
            }
        }
        self.largest_sources.push(ScriptSourceMemorySample {
            url: script.url.as_str().to_owned(),
            source_bytes: source_len,
            kind: script.kind,
            mode: script.mode,
            source_kind: script.source_kind,
        });
        self.largest_sources.sort_by(|left, right| {
            right
                .source_bytes
                .cmp(&left.source_bytes)
                .then_with(|| left.url.cmp(&right.url))
        });
        self.largest_sources.truncate(12);
    }

    fn to_diagnostics(&self) -> RendererScriptExecutionMemoryDiagnostics {
        RendererScriptExecutionMemoryDiagnostics {
            execution_count: self.execution_count,
            total_source_bytes: self.total_source_bytes,
            inline_source_bytes: self.inline_source_bytes,
            external_source_bytes: self.external_source_bytes,
            classic_source_bytes: self.classic_source_bytes,
            module_source_bytes: self.module_source_bytes,
            import_map_source_bytes: self.import_map_source_bytes,
            data_block_source_bytes: self.data_block_source_bytes,
            inline_execution_count: self.inline_execution_count,
            external_execution_count: self.external_execution_count,
            classic_execution_count: self.classic_execution_count,
            module_execution_count: self.module_execution_count,
            import_map_execution_count: self.import_map_execution_count,
            data_block_execution_count: self.data_block_execution_count,
            largest_sources: self
                .largest_sources
                .iter()
                .map(|sample| RendererScriptSourceMemoryDiagnostics {
                    url: sample.url.clone(),
                    source_bytes: sample.source_bytes,
                    kind: format!("{:?}", sample.kind),
                    mode: format!("{:?}", sample.mode),
                    source_kind: format!("{:?}", sample.source_kind),
                })
                .collect(),
        }
    }
}

use super::native_bridge::element::ClientRect;
use super::{
    context_bootstrap::{set_window_navigator_identity, sync_global_location_runtime_state},
    custom_elements,
    document_runtime::{CurrentScriptContextSpec, DocumentRuntime, DomHandle},
    dom::native::ShadowRootInclusion,
    host::ScriptHandleSource,
    host::{HostTimeoutRunResult, ScriptEventKind, ScriptEventTask},
    module_runtime::{
        ModuleScriptExecutionOutcome, ModuleSource, execute_external_module_script_graph,
        execute_module_script_source, register_import_map_source,
    },
    native_bridge::{
        JsContextHost, JsContextHostBridgeRef, RuntimeObservableContextToken,
        SharedPrebootstrappedChildDefaultContexts, node_runtime_and_handle_from_object,
    },
    planning::PreparedScript,
    renderer_resource_scheduler::RendererResourceScheduler,
    runtime::{
        RendererActivityDiagnostics, RendererInputDispatchOutcome, RendererPageContextCancelReason,
        RendererPageContextCancelSender, RendererPageDiagnosticsSnapshot,
        RendererRuntimeObservableSourceQueue, renderer_page_context_cancel_channel,
    },
    types::{JsValueSnapshot, ScriptKind, ScriptMode, ScriptSourceKind, SubresourceResourceType},
    util::v8_string,
};

#[cfg(test)]
use super::native_bridge::PendingRuntimeBindingCall;
#[cfg(test)]
use super::types::{PendingSubresourceContinueEvent, PendingSubresourceFetchInfo};

use crate::module_script_continuation::{
    ModuleScriptCompletionOwner, ModuleScriptContinuation, ModuleScriptContinuationGraphAdvance,
};

mod app_manifest;
mod autofill;
mod blob_inspector;
mod broadcast_channel_delivery;
mod child_classic_document_script;
mod child_classic_source_load;
mod child_document_event;
mod child_document_lifecycle;
pub(crate) use child_document_lifecycle::ChildDocumentLifecycleRunOutcome;
mod child_document_modulator;
mod child_document_script_owner_hooks;
mod child_document_script_scheduler;
mod child_document_script_task_effect;
pub(crate) use child_document_script_task_effect::{
    ChildDocumentScriptActivity, ChildDocumentScriptReadyRunOutcome, ChildDocumentScriptRunOutcome,
};
mod child_dynamic_document_script;
mod child_frame_realm;
mod child_frame_realm_materialization;
mod child_realm_materialization_completion;
mod classic_script_exception;
pub(crate) use child_frame_realm_materialization::{
    ChildRealmMaterializationApplication, ChildRealmMaterializationBodyActivity,
};
mod child_host_load;
pub(crate) use child_host_load::ChildHostLoadRunOutcome;
mod child_module_fetch;
mod child_module_script_terminal;
mod child_module_script_terminal_batch;
mod child_modulepreload_event_action;
mod child_navigation_commit;
mod context_scope;
mod dedicated_worker_client_event_body;
#[cfg(test)]
mod dedicated_worker_client_event_test_support;
mod dedicated_worker_error_dispatch;
mod devtools_resource_load;
mod directory_reader_callback;
mod document_content;
mod document_isolate;
mod dom_debugger;
mod dom_inspector;
pub(crate) use dom_inspector::{DomInspectorEdit, DomInspectorEditOutcome};
mod drop_cleanup;
mod element_click;
mod element_toggle_event;
mod eval_exec;
mod file_entry_file_callback;
mod frame_script_jobs;
mod hash_change_delivery;
mod history_traversal;
mod image_load_event;
mod indexed_db_task_body;
mod input_dispatch;
mod input_helpers;
mod inspector;
pub(crate) use inspector::{dispatch_inspector_io_owner_wake, dispatch_inspector_main_owner_wake};
mod isolated_worlds;
mod main_document_lifecycle;
mod main_document_lifecycle_body;
mod main_document_lifecycle_completion;
#[cfg(test)]
mod main_document_lifecycle_test_support;
mod main_document_owner;
mod main_document_post_parse_body;
mod main_document_post_parse_completion;
mod main_document_script_completion;
mod main_parser_classic_completion;
mod main_parser_continuation_completion;
mod media_element_event;
mod message_port_delivery;
mod misc_platform_api;
mod native_module;
mod navigation_api_task;
mod navigation_history;
#[cfg(test)]
mod opfs_task_test_support;
mod opfs_tasks;
mod page_resource_completion_owner;
mod page_task_capabilities;
mod page_task_enqueue;
mod parser_owned_classic;
pub(crate) use parser_owned_classic::*;
mod parser_module_terminal;
mod popup_load_event;
mod post_parse;
mod post_parse_lifecycle;
mod script_event_body;
mod script_terminal_completion;
pub(crate) use post_parse_lifecycle::RuntimeOwnedModuleFailureBodySettlement;
mod browsing_contexts;
mod child_frame_runtime;
mod document_environment;
mod execution_contexts;
mod layout;
mod prepared_scripts;
mod rendering_update;
mod runtime_bindings;
mod runtime_evaluation;
mod runtime_observability;
mod runtime_script_continuation;
mod script_tasks;
pub(crate) use runtime_script_continuation::RuntimeScriptContinuationBodyEffect;
#[cfg(test)]
pub(crate) use runtime_script_continuation::RuntimeScriptOwnerAdvance;
mod security_policy;
mod service_worker_client_message_body;
#[cfg(test)]
mod service_worker_client_message_test_support;
mod service_worker_internal_body;
mod service_worker_internal_client_request_body;
mod service_worker_internal_event_body;
mod service_worker_internal_promise_body;
#[cfg(test)]
mod service_worker_internal_test_support;
mod service_workers;
mod shared_worker_client_event_body;
#[cfg(test)]
mod shared_worker_client_event_test_support;
mod storage_event_delivery;
#[cfg(test)]
mod stylesheet_page_task_test_support;
mod stylesheet_page_tasks;
mod subresource_command_completion;
mod subresource_fetch;
pub(crate) use subresource_command_completion::AsyncSubresourceCommandExecution;
pub(crate) use subresource_fetch::AsyncSubresourceFetchBodyActivity;
mod page_resource_completion_task_completion;
mod text_search;
mod text_track_default_mode;
mod text_track_load;
mod user_interaction;
mod view_transition_update;
pub(crate) mod web_fonts;
pub(crate) mod webcrypto_tasks;
mod websocket_event_body;
mod websocket_worker;
mod window_message;
mod worker_host_bridge_body;

pub(crate) use dedicated_worker_client_event_body::DedicatedWorkerClientEventBodyEffect;
#[cfg(test)]
pub(crate) use indexed_db_task_body::IndexedDbStaleTaskCleanupEffect;
pub(crate) use indexed_db_task_body::IndexedDbTaskBodyEffect;
pub(crate) use main_document_lifecycle::{
    MainDocumentLifecycleBody, MainDocumentLifecycleBodyKind, MainDocumentLifecycleCallbackEffect,
    MainDocumentLifecycleCheckpoint, MainDocumentLifecycleCompletion,
    MainDocumentLifecycleEventDispatch, MainDocumentLifecycleExecution,
    MainDocumentLifecycleFailure, MainDocumentLifecycleFollowup, MainDocumentLifecycleStep,
    MainDocumentLifecycleTargetEffect, MainDocumentLifecycleTargetRejection,
};
pub(crate) use native_module::{
    MainDynamicImportGraphFetchBodySettlement, MainNativeModuleSelectedTaskApplication,
    MainNativeModuleSelectedTaskBodyActivity,
};
pub(crate) use navigation_api_task::NavigationApiTaskBodyApplied;
pub(crate) use service_worker_client_message_body::{
    ServiceWorkerClientMessageBodyCallbackEffect, ServiceWorkerClientMessageBodyEffect,
    ServiceWorkerClientMessageBodyEventKind,
};
pub(crate) use service_worker_internal_body::{
    ServiceWorkerInternalBodyCallbackEffect, ServiceWorkerInternalBodyEffect,
};
pub(crate) use shared_worker_client_event_body::{
    SharedWorkerClientEventBodyEffect, SharedWorkerErrorDispatchEffect,
};
pub(crate) use worker_host_bridge_body::WorkerHostBridgeBodyEffect;

pub(crate) use post_parse::bootstrap_child_default_context_in_scope;

fn input_dispatch_outcome(handled: bool) -> RendererInputDispatchOutcome {
    RendererInputDispatchOutcome {
        handled,
        triggered_top_level_navigation: false,
        pending_download: None,
        pending_file_chooser: None,
    }
}

#[cfg(test)]
mod detached_document_native_handle_tests;
#[cfg(test)]
mod dom_heavy_regression_tests;
pub(crate) mod runtime_work;
#[cfg(test)]
mod standalone_test_harness;

pub(crate) use parser_module_terminal::{
    ParserModuleEvaluationSettlement, ParserModuleTerminalDisposition,
    ParserOwnedModuleSuccessTerminal, PreparedModuleSuccessSettlement,
};
#[cfg(test)]
mod tests;
#[cfg(test)]
mod traversal_tests;
#[cfg(test)]
pub(crate) use standalone_test_harness::StandaloneScriptVmHarness;

use crate::document_runtime::{DeferredPageTaskLane, FollowupPageTaskDisposition};
use document_isolate::*;
pub(crate) use document_isolate::{
    RendererDeferredContextHostReleaseQueue, RendererDocumentIsolateBootstrap,
    RendererDocumentIsolateHandle, RendererDocumentIsolateReservationAccounting,
    RendererPageScriptEnvironment, ScriptVmDefaultWorldBootstrap,
    renderer_document_isolate_accounting_diagnostics,
};
pub(crate) use eval_exec::execute_source_text_on_current_stack;
pub(crate) use input_helpers::*;
use inspector::*;
pub(crate) use inspector::{
    DocumentInspectorBinding, RendererDomDebuggerPauseScheduler, RendererDomDebuggerScheduledPause,
};
use isolated_worlds::*;
pub(crate) use runtime_bindings::PromiseRejectDispatchSlot;
pub(crate) use runtime_bindings::perform_microtask_checkpoint_and_report_pending_promise_rejections;
use runtime_bindings::*;
pub(crate) use runtime_work::*;
use std::ops::{Deref, DerefMut};

#[cfg(any(test, feature = "test-support"))]
type ScriptGlobalsBaseline = Vec<String>;
#[cfg(not(any(test, feature = "test-support")))]
struct ScriptGlobalsBaseline;

fn recover_bootstrap_dom_host_from_holder(
    renderer_document_isolate: RendererDocumentIsolateHandle,
    renderer_document_isolate_teardown: RendererDocumentIsolateTeardown,
    context_host: Rc<RefCell<JsContextHost>>,
    document_runtime: Box<DocumentRuntime>,
) -> DomHost {
    drop(context_host);
    renderer_document_isolate_teardown
        .unregister_platform_on_context_teardown(&renderer_document_isolate);
    drop(renderer_document_isolate);
    document_runtime.into_dom_host()
}

fn register_main_window_execution_context_for_bootstrap(
    renderer_document_isolate: &RendererDocumentIsolateHandle,
    context_host: &Rc<RefCell<JsContextHost>>,
    context: &v8::Global<v8::Context>,
) -> Result<()> {
    renderer_document_isolate
        .with_entered_renderer_document_isolate(|isolate| {
            let scope = pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = v8::Local::new(scope, context);
            let scope = &mut v8::ContextScope::new(scope, context);
            let host = &mut *context_host.borrow_mut();
            let binding = host
                .current_window_execution_context_binding(
                    scope,
                    crate::native_bridge::OwnerDispatchScope::Top,
                )
                .ok_or_else(|| anyhow!("main LocalWindow execution context is unavailable"))?;
            host.register_window_execution_context(binding);
            Ok(())
        })
        .context("failed to register main LocalWindow execution context")
}

pub(super) struct ScriptVmDocumentRuntimeOwner {
    document_runtime: Option<Box<DocumentRuntime>>,
}

impl ScriptVmDocumentRuntimeOwner {
    fn new(document_runtime: Box<DocumentRuntime>) -> Self {
        Self {
            document_runtime: Some(document_runtime),
        }
    }

    fn take_for_retained_document_host(&mut self) -> Option<Box<DocumentRuntime>> {
        self.document_runtime.take()
    }
}

impl Deref for ScriptVmDocumentRuntimeOwner {
    type Target = DocumentRuntime;

    fn deref(&self) -> &Self::Target {
        self.document_runtime
            .as_deref()
            .expect("ScriptVm DocumentRuntime must remain owned until ScriptVm drop")
    }
}

impl DerefMut for ScriptVmDocumentRuntimeOwner {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.document_runtime
            .as_deref_mut()
            .expect("ScriptVm DocumentRuntime must remain owned until ScriptVm drop")
    }
}

pub(super) struct ScriptVm {
    resource_owner_id: crate::resource_owner::ResourceOwnerId,
    /// Page/target-facing inspector state. This must drop before the renderer
    /// document isolate handle because the V8 inspector session touches the
    /// isolate-level backend while being destroyed.
    page_inspector: DocumentInspectorBinding,
    renderer_document_isolate_teardown: RendererDocumentIsolateTeardown,
    renderer_page_script_environment: Option<RendererPageScriptEnvironment>,
    page_default_context: v8::Global<v8::Context>,
    page_default_bridge_ref: Option<JsContextHostBridgeRef>,
    page_isolated_world_contexts: PageIsolatedWorldRegistry,
    child_frame_realm_store: child_frame_realm::ChildFrameRealmStore,
    prebootstrapped_child_default_contexts: SharedPrebootstrappedChildDefaultContexts,
    child_document_modulator_store: ChildDocumentModulatorStore,
    page_default_runtime_observable_context_token: RuntimeObservableContextToken,
    root_frame_id: Option<String>,
    baseline_globals: ScriptGlobalsBaseline,
    // `JsContextHost` stores a non-owning pointer into `document_runtime`, so it
    // must be dropped before the runtime field during normal Rust field teardown.
    _context_host: Rc<RefCell<JsContextHost>>,
    pub(super) document_runtime: ScriptVmDocumentRuntimeOwner,
    post_domcontentloaded_page_task_tx: PageTaskSender,
    page_runtime_wake_tx: PageRuntimeWakeSender,
    queued_main_document_runtime_continuation_owner:
        Option<crate::frame_owner_model::FrameDocumentTaskOwner>,
    queued_main_document_module_continuation_owner:
        Option<crate::frame_owner_model::FrameDocumentTaskOwner>,
    queued_main_document_parser_module_continuation_owner:
        Option<crate::frame_owner_model::FrameDocumentTaskOwner>,
    script_execution_memory: ScriptExecutionMemoryCounters,
    runtime_observable_source_queue: RendererRuntimeObservableSourceQueue,
    page_context_cancel_tx: RendererPageContextCancelSender,
    pressed_mouse_buttons: i32,
    pending_mouse_press: Option<PendingMousePress>,
    hovered_mouse_handle: Option<DomHandle>,
    /// Root-frame to local-frame transform for `hovered_mouse_handle`. Blink
    /// keeps this conversion on LocalFrameView; retaining the last affine map
    /// lets exit events use the old frame after a new hit enters another one.
    hovered_mouse_root_to_frame: moli_layout::LayoutTransform2D,
    active_touch_pointer_handle: Option<DomHandle>,
    active_touch_pointer_handles: BTreeMap<i32, DomHandle>,
    active_touch_event_handle: Option<DomHandle>,
    active_touch_point: Option<crate::runtime::RendererTouchPoint>,
    active_touch_points: BTreeMap<i32, ActiveTouchPoint>,
    suppress_next_keypress_after_canceled_raw_keydown: bool,
    suppress_compat_mouse_events: bool,
    active_scrollbar_drag: Option<ActiveScrollbarDrag>,
    active_drag_session: Option<ActiveDragSession>,
    promise_reject_dispatch: PromiseRejectDispatchSlot,
    next_internal_runtime_evaluate_call_id: i32,
    next_internal_frontend_inspector_call_id: i32,
    pending_internal_runtime_evaluates:
        HashMap<i32, tokio::sync::oneshot::Receiver<RendererRuntimeInspectorAsyncCompletion>>,
    indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
    storage_bucket_store: crate::context_bootstrap::SharedStorageBucketStore,
    app_manifest_cache: Option<app_manifest::ScriptVmAppManifestCache>,
    #[cfg(test)]
    test_next_timeout_failure: Option<String>,
    #[cfg(test)]
    _page_task_residence_for_executor_test:
        Option<crate::page_task_queue::RendererPageTaskTestResidence>,
    // Native and V8 per-document state must drop before the final isolate
    // handle; retained realms then release their host through its GC queue.
    renderer_document_isolate: RendererDocumentIsolateHandle,
}

impl moli_layout::GeometryProvider for ScriptVm {
    type NodeId = DomHandle;

    fn answer(
        &mut self,
        queries: &moli_layout::LayoutQueryBatch<Self::NodeId>,
    ) -> Result<moli_layout::LayoutAnswers<Self::NodeId>, moli_layout::LayoutError> {
        self._context_host.borrow_mut().answer(queries)
    }
}

pub(super) struct ScriptVmCommandTurnOutputScope {
    context_host: Rc<RefCell<JsContextHost>>,
    recorder: crate::runtime::RendererCommandTurnOutputRecorder,
    _inspector_scope: ScriptVmInspectorCommandTurnOutputScope,
}

pub(super) struct ScriptVmOrdinaryPageTurnNavigationHandoffScope {
    context_host: Rc<RefCell<JsContextHost>>,
}

impl Drop for ScriptVmOrdinaryPageTurnNavigationHandoffScope {
    fn drop(&mut self) {
        self.context_host
            .borrow_mut()
            .end_ordinary_page_turn_navigation_handoff();
    }
}

impl Drop for ScriptVmCommandTurnOutputScope {
    fn drop(&mut self) {
        self.context_host
            .borrow_mut()
            .end_command_turn_output(&self.recorder);
    }
}

struct LiveChildDefaultContextEntry {
    handle: DomHandle,
    frame_id: String,
    owner_realm_id: Option<FrameRealmId>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ActiveTouchPoint {
    pub x: f64,
    pub y: f64,
    pub target: DomHandle,
}

pub(super) struct ActiveDragSession {
    pub data_transfer: v8::Global<v8::Object>,
    pub drop_allowed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ActiveScrollbarDrag {
    pub source: DomHandle,
    pub scrollbar: moli_layout::LayoutScrollbarGeometry,
    pub pointer_origin: f32,
    pub viewport_to_local: moli_layout::LayoutTransform2D,
}

struct PageRuntimeObservableContext {
    execution_context_id: Option<i64>,
    context_token: RuntimeObservableContextToken,
    context: *const v8::Global<v8::Context>,
}

pub(super) struct ScriptVmRendererDocumentIsolateOps<'a> {
    vm: &'a mut ScriptVm,
}

impl ScriptVm {
    pub(crate) fn devtools_target(&self) -> crate::devtools::target::RendererDevToolsTargetHandle {
        self.page_inspector.devtools_target()
    }

    pub(super) fn set_root_document_lifecycle(
        &mut self,
        lifecycle: crate::runtime::RendererDocumentLifecycleJournalHandle,
    ) {
        self._context_host
            .borrow_mut()
            .set_root_document_lifecycle(lifecycle);
    }

    pub(super) fn run_v8_foreground_task(
        &mut self,
        task: moli_v8_platform::V8ForegroundTask,
    ) -> bool {
        self.renderer_document_isolate
            .with_renderer_document_isolate_mut(|_| task.run())
    }

    #[cfg(test)]
    pub(crate) fn has_pending_image_network_requests(&self) -> bool {
        self._context_host
            .borrow()
            .has_pending_image_network_requests()
    }

    #[cfg(test)]
    pub(crate) fn has_pending_load_event_delaying_subresource_requests(&self) -> bool {
        self._context_host
            .borrow()
            .has_pending_load_event_delaying_subresource_requests()
    }

    pub(crate) fn with_dom_host_parse_step<R>(&mut self, step: impl FnOnce(&mut Self) -> R) -> R {
        struct FinishParserStepOnDrop<'a> {
            vm: &'a mut ScriptVm,
        }

        impl Drop for FinishParserStepOnDrop<'_> {
            fn drop(&mut self) {
                self.vm.document_runtime.finish_dom_host_parse_step();
            }
        }

        self.document_runtime.begin_dom_host_parse_step();
        let guard = FinishParserStepOnDrop { vm: self };
        step(&mut *guard.vm)
    }

    pub(super) fn set_indexed_db_manager(
        &mut self,
        manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
    ) {
        self.indexed_db_manager = manager.clone();
        self._context_host
            .borrow_mut()
            .set_indexed_db_manager(manager.clone());
        let mut context_ptrs: Vec<*const v8::Global<v8::Context>> = Vec::with_capacity(
            1 + self.page_isolated_world_contexts.len() + self.child_frame_realm_store.len(),
        );
        context_ptrs.push(&self.page_default_context as *const _);
        context_ptrs.extend(
            self.page_isolated_world_contexts
                .contexts()
                .map(|world| &world.context as *const _),
        );
        context_ptrs.extend(
            self.child_frame_realm_store
                .values()
                .map(|world| &world.context as *const _),
        );

        let _ = self
            .renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                for context_ptr in context_ptrs {
                    let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                    crate::context_bootstrap::set_indexed_db_manager_for_context(
                        context,
                        manager.clone(),
                    );
                }
                Ok(())
            });
    }

    pub(super) fn set_storage_bucket_store(
        &mut self,
        store: crate::context_bootstrap::SharedStorageBucketStore,
    ) {
        self.storage_bucket_store = store.clone();
        self._context_host
            .borrow_mut()
            .set_storage_bucket_store(store.clone());
        let mut context_ptrs: Vec<*const v8::Global<v8::Context>> = Vec::with_capacity(
            1 + self.page_isolated_world_contexts.len() + self.child_frame_realm_store.len(),
        );
        context_ptrs.push(&self.page_default_context as *const _);
        context_ptrs.extend(
            self.page_isolated_world_contexts
                .contexts()
                .map(|world| &world.context as *const _),
        );
        context_ptrs.extend(
            self.child_frame_realm_store
                .values()
                .map(|world| &world.context as *const _),
        );

        let _ = self
            .renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                for context_ptr in context_ptrs {
                    let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                    crate::context_bootstrap::set_storage_bucket_store_for_context(
                        context,
                        Some(store.clone()),
                    );
                }
                Ok(())
            });
    }

    #[cfg(test)]
    pub(super) fn dispatch_inspector_protocol_message(
        &mut self,
        raw_json: &str,
    ) -> Result<Vec<Value>> {
        self.dispatch_inspector_protocol_message_for_session(None, raw_json)
            .map(|messages| {
                messages
                    .into_iter()
                    .map(RendererRuntimeInspectorMessage::into_v8_inspector_message)
                    .collect()
            })
    }

    pub(super) fn dispatch_inspector_protocol_message_for_session(
        &mut self,
        inspector_session_id: Option<&str>,
        raw_json: &str,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        self.dispatch_inspector_protocol_message_for_session_with_optional_deferred_response(
            PageInspectorSessionTarget::Frontend(inspector_session_id),
            raw_json,
            None,
            None,
            None,
        )
    }

    /// Dispatches a renderer-owned Inspector command whose response is an
    /// implementation detail rather than frontend protocol output.
    ///
    /// Runtime.enable is used here to ask V8 for the authoritative live-context
    /// replay. Its notifications belong to the command-local replay returned
    /// by this call, while the synthetic response ID must never enter the
    /// Page's concrete output stream. Marking the dispatch internal also keeps
    /// the replay from becoming a second live Runtime producer.
    pub(super) fn dispatch_internal_inspector_protocol_message_for_session(
        &mut self,
        inspector_session_id: Option<&str>,
        raw_json: &str,
        internal_call_id: i32,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        self.dispatch_inspector_protocol_message_for_session_with_optional_deferred_response(
            PageInspectorSessionTarget::Frontend(inspector_session_id),
            raw_json,
            None,
            None,
            Some(internal_call_id),
        )
    }

    pub(super) fn live_node_handle_for_runtime_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
    ) -> Result<Option<DomHandle>> {
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        let context_host = self._context_host.clone();
        let page_inspector = &self.page_inspector;
        let renderer_document_isolate = self.renderer_document_isolate.clone();
        renderer_document_isolate.with_entered_renderer_document_isolate_and_inspector_mut(
            |isolate, inspector| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let expected_runtime_ptr: *mut JsContextHost = (*context_host).as_ptr();
                page_inspector.with_session_and_outbound(
                    inspector,
                    PageInspectorSessionTarget::Frontend(inspector_session_id),
                    |session, _, _| -> Result<Option<DomHandle>> {
                        let Ok((value, context, _object_group)) = session.unwrap_object(
                            scope,
                            v8::inspector::StringView::from(object_id.as_bytes()),
                        ) else {
                            return Ok(None);
                        };
                        let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
                            return Ok(None);
                        };
                        let scope = &mut v8::ContextScope::new(scope, context);
                        let Ok((runtime_ptr, handle)) =
                            node_runtime_and_handle_from_object(scope, object)
                        else {
                            return Ok(None);
                        };
                        if runtime_ptr != expected_runtime_ptr
                            || unsafe { &*runtime_ptr }.dom_host().node(handle).is_none()
                        {
                            return Ok(None);
                        }
                        Ok(Some(handle))
                    },
                )
            },
        )
    }

    pub(super) fn child_frame_id_for_live_node_handle(&self, handle: DomHandle) -> Option<String> {
        let host = self._context_host.borrow();
        let handle = host.dom_host().document_identity_handle(handle)?;
        let document_handle = host.dom_host().owner_document_handle(handle)?;
        if document_handle == host.dom_host().document_handle() {
            return None;
        }
        let child_handle = host.child_browsing_context_host_for_document_handle(document_handle)?;
        host.frame_owner_frame_id_for_child_handle(child_handle)
            .map(|frame_id| frame_id.0)
    }

    pub(super) fn document_id_for_live_node_handle(&self, handle: DomHandle) -> Option<DocumentId> {
        let host = self._context_host.borrow();
        let handle = host.dom_host().document_identity_handle(handle)?;
        let document_handle = host.dom_host().owner_document_handle(handle)?;
        if document_handle == host.dom_host().document_handle() {
            return host
                .current_main_document_task_owner()
                .map(|owner| owner.document_id);
        }
        let child_handle = host.child_browsing_context_host_for_document_handle(document_handle)?;
        host.frame_owner_current_child_snapshot(child_handle)
            .map(|snapshot| snapshot.document_id)
    }

    pub(super) fn outer_html_for_live_node_handle(
        &self,
        handle: DomHandle,
        include_shadow_dom: bool,
    ) -> Option<String> {
        let host = self._context_host.borrow();
        let shadow_root_inclusion = if include_shadow_dom {
            ShadowRootInclusion::AllAuthorForInspector
        } else {
            ShadowRootInclusion::None
        };
        let should_serialize_registry_attribute =
            |_: DomHandle, shadow_root: DomHandle, _: &crate::dom::native::ShadowRootInit| {
                host.should_serialize_shadow_root_registry_attribute(shadow_root)
            };
        let scripting_enabled_for_node = |node| host.node_document_scripting_enabled(node);
        host.dom_host().outer_html_with_shadow_roots(
            handle,
            &scripting_enabled_for_node,
            shadow_root_inclusion,
            Some(&should_serialize_registry_attribute),
        )
    }

    pub(crate) fn renderer_dom_agent_state(&self) -> crate::runtime::RendererDomAgentState {
        self._context_host.borrow().renderer_dom_agent_state()
    }

    pub(super) fn dispatch_inspector_protocol_message_for_session_with_deferred_response_and_command_output(
        &mut self,
        inspector_session_id: Option<&str>,
        raw_json: &str,
        deferred_response: RendererRuntimeInspectorResponseSender,
        command_output: crate::runtime::RendererRuntimeCommandOutputRecorder,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        self.dispatch_inspector_protocol_message_for_session_with_optional_deferred_response(
            PageInspectorSessionTarget::Frontend(inspector_session_id),
            raw_json,
            Some(deferred_response),
            Some(command_output),
            None,
        )
    }

    pub(super) fn route_frontend_runtime_inspector_response(
        &self,
        inspector_session_id: Option<&str>,
        response: RendererRuntimeInspectorResponseSender,
    ) -> Result<RendererRuntimeInspectorResponseSender> {
        self.page_inspector
            .route_frontend_response(inspector_session_id, response)
    }

    fn dispatch_internal_runtime_evaluate_protocol_message(
        &mut self,
        raw_json: &str,
        deferred_response: RendererRuntimeInspectorResponseSender,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        self.dispatch_inspector_protocol_message_for_session_with_optional_deferred_response(
            PageInspectorSessionTarget::InternalRuntimeEvaluate,
            raw_json,
            Some(deferred_response),
            None,
            None,
        )
    }

    pub(super) fn end_runtime_inspector_command_output(&self, inspector_session_id: Option<&str>) {
        self.page_inspector
            .end_runtime_command_output_for_session(inspector_session_id);
    }

    pub(super) fn begin_command_turn_output(
        &self,
        recorder: crate::runtime::RendererCommandTurnOutputRecorder,
    ) -> Result<ScriptVmCommandTurnOutputScope> {
        self._context_host
            .borrow_mut()
            .begin_command_turn_output(recorder.clone())?;
        let inspector_scope = match self
            .page_inspector
            .begin_command_turn_output(recorder.clone())
        {
            Ok(scope) => scope,
            Err(error) => {
                self._context_host
                    .borrow_mut()
                    .end_command_turn_output(&recorder);
                return Err(error);
            }
        };
        Ok(ScriptVmCommandTurnOutputScope {
            context_host: Rc::clone(&self._context_host),
            recorder,
            _inspector_scope: inspector_scope,
        })
    }

    pub(super) fn begin_ordinary_page_turn_navigation_handoff(
        &self,
    ) -> Result<ScriptVmOrdinaryPageTurnNavigationHandoffScope> {
        self._context_host
            .borrow_mut()
            .begin_ordinary_page_turn_navigation_handoff()?;
        Ok(ScriptVmOrdinaryPageTurnNavigationHandoffScope {
            context_host: Rc::clone(&self._context_host),
        })
    }

    pub(super) fn cancel_runtime_inspector_response_for_session(
        &self,
        inspector_session_id: Option<&str>,
        call_id: i32,
    ) {
        self.page_inspector
            .cancel_response_callback_for_session(inspector_session_id, call_id);
    }

    pub(super) fn initialize_inspector_session_after_attach(
        &mut self,
        inspector_session_id: Option<&str>,
        protocol_configuration: &RendererInspectorProtocolConfiguration,
        v8_attach: &V8InspectorSessionAttach,
    ) -> Result<()> {
        const INTERNAL_RUNTIME_ENABLE_ID: u64 = 900_013;
        const INTERNAL_CONSOLE_ENABLE_ID: u64 = 900_014;

        self.set_inspector_session_runtime_bindings(
            inspector_session_id,
            &protocol_configuration.runtime_bindings,
        );
        for breakpoint in &protocol_configuration.dom_debugger_event_listener_breakpoints {
            self.configure_dom_debugger_event_listener_breakpoint(
                inspector_session_id,
                breakpoint.clone(),
                true,
            );
        }
        for breakpoint in &protocol_configuration.dom_debugger_xhr_breakpoints {
            self.configure_dom_debugger_xhr_breakpoint(
                inspector_session_id,
                breakpoint.clone(),
                true,
            );
        }
        let is_first_attach = matches!(v8_attach, V8InspectorSessionAttach::FirstAttach);
        let mut first_attach_bootstrap_commands = Vec::new();
        if is_first_attach && protocol_configuration.runtime_frontend_enabled {
            first_attach_bootstrap_commands.push((
                INTERNAL_RUNTIME_ENABLE_ID,
                "Runtime.enable",
                serde_json::to_string(&json!({
                    "id": INTERNAL_RUNTIME_ENABLE_ID,
                    "method": "Runtime.enable",
                }))?,
            ));
        }
        if is_first_attach && protocol_configuration.console_frontend_enabled {
            first_attach_bootstrap_commands.push((
                INTERNAL_CONSOLE_ENABLE_ID,
                "Console.enable",
                serde_json::to_string(&json!({
                    "id": INTERNAL_CONSOLE_ENABLE_ID,
                    "method": "Console.enable",
                }))?,
            ));
        }
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        let runtime_binding_replay_context_ptrs = self.runtime_binding_replay_context_ptrs();
        let renderer_document_isolate = self.renderer_document_isolate.clone();
        let page_inspector = &mut self.page_inspector;
        renderer_document_isolate.with_entered_renderer_document_isolate_and_inspector_mut(
            |isolate, inspector| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                page_inspector.with_session_and_outbound(
                    inspector,
                    PageInspectorSessionTarget::Frontend(inspector_session_id),
                    |session, outbound, runtime_bindings_to_replay| -> Result<()> {
                        let replay_global_snapshots =
                            capture_runtime_binding_replay_global_snapshots(
                                scope,
                                &runtime_binding_replay_context_ptrs,
                                &runtime_bindings_to_replay,
                            );
                        for (index, binding) in runtime_bindings_to_replay.iter().enumerate() {
                            let (replay_call_id, replay_request) =
                                runtime_binding_replay_request_json(binding, index)?;
                            let replay_snap = outbound.len();
                            {
                                let _internal_response_capture =
                                    outbound.capture_internal_dispatch_response(replay_call_id);
                                let _dispatch_response_capture =
                                    outbound.capture_dispatch_responses();
                                with_scoped_inspector_microtasks(scope, || {
                                    session.dispatch_protocol_message(
                                        v8::inspector::StringView::from(replay_request.as_bytes()),
                                    );
                                });
                            }
                            outbound.discard_messages_after(replay_snap);
                        }
                        restore_runtime_binding_replay_global_snapshots(
                            scope,
                            replay_global_snapshots,
                        );
                        for (call_id, method, raw_json) in &first_attach_bootstrap_commands {
                            let restore_snapshot = outbound.len();
                            {
                                let _internal_response_capture = outbound
                                    .capture_internal_dispatch_response(
                                        i32::try_from(*call_id)
                                            .expect("bounded inspector restore call id"),
                                    );
                                let _dispatch_response_capture =
                                    outbound.capture_dispatch_responses();
                                with_scoped_inspector_microtasks(scope, || {
                                    dispatch_with_runtime_defaults(session, raw_json, &outbound)
                                })
                                .map_err(anyhow::Error::msg)?;
                            }
                            let response = outbound
                                .take_response_for_call_id_after(
                                    restore_snapshot,
                                    i64::try_from(*call_id)
                                        .expect("bounded inspector restore call id"),
                                )
                                .ok_or_else(|| {
                                    anyhow!("{method} restore produced no inspector response")
                                })?;
                            if let Some(error) = response.get("error") {
                                return Err(anyhow!("{method} restore failed: {error}"));
                            }
                            outbound.append_messages_after_to_output_journal(restore_snapshot)?;
                        }
                        Ok(())
                    },
                )
            },
        )?;
        self.sync_child_browsing_context_records();
        self.finish_runtime_turn_with_style_drain(
            crate::style_engine::StyleInvalidationTurnExitBoundary::RuntimeEvaluate,
            (),
        );
        Ok(())
    }

    pub(super) fn configure_dom_debugger_event_listener_breakpoint(
        &mut self,
        inspector_session_id: Option<&str>,
        breakpoint: RendererDomDebuggerEventListenerBreakpoint,
        enabled: bool,
    ) {
        self._context_host
            .borrow_mut()
            .configure_dom_debugger_event_listener_breakpoint(
                inspector_session_id,
                breakpoint,
                enabled,
            );
    }

    pub(super) fn configure_dom_debugger_xhr_breakpoint(
        &mut self,
        inspector_session_id: Option<&str>,
        breakpoint: RendererDomDebuggerXhrBreakpoint,
        enabled: bool,
    ) {
        self._context_host
            .borrow_mut()
            .configure_dom_debugger_xhr_breakpoint(inspector_session_id, breakpoint, enabled);
    }

    pub(super) fn configure_dom_debugger_dom_breakpoint(
        &mut self,
        inspector_session_id: Option<&str>,
        document_id: DocumentId,
        handle: DomHandle,
        breakpoint_type: RendererDomDebuggerDomBreakpointType,
        enabled: bool,
    ) {
        self._context_host
            .borrow_mut()
            .configure_dom_debugger_dom_breakpoint(
                inspector_session_id,
                document_id,
                handle,
                breakpoint_type,
                enabled,
            );
    }

    pub(super) fn clear_dom_debugger_dom_breakpoints_for_session(
        &mut self,
        inspector_session_id: Option<&str>,
    ) {
        self._context_host
            .borrow_mut()
            .clear_dom_debugger_dom_breakpoints_for_session(inspector_session_id);
    }

    fn dispatch_inspector_protocol_message_for_session_with_optional_deferred_response(
        &mut self,
        target: PageInspectorSessionTarget<'_>,
        raw_json: &str,
        deferred_response: Option<RendererRuntimeInspectorResponseSender>,
        command_output: Option<crate::runtime::RendererRuntimeCommandOutputRecorder>,
        internal_dispatch_call_id: Option<i32>,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        let user_gesture = runtime_protocol_message_user_gesture(raw_json);
        let file_prompt_handler = runtime_protocol_message_file_prompt_handler(raw_json);
        if user_gesture {
            self._context_host
                .borrow_mut()
                .begin_protocol_user_gesture_activation();
        }
        if let Some(handler) = file_prompt_handler.as_deref() {
            self._context_host
                .borrow_mut()
                .begin_webdriver_bidi_file_prompt_handler(handler);
        }
        let runtime_command_cause = command_output
            .as_ref()
            .map(|recorder| recorder.causal_identity());
        let previous_runtime_command_cause = self
            ._context_host
            .borrow_mut()
            .replace_active_runtime_command_cause(runtime_command_cause.clone());
        let previous_inspector_dispatch = self
            ._context_host
            .borrow_mut()
            .replace_active_inspector_dispatch(true);
        let result = self.dispatch_inspector_protocol_message_with_current_activation(
            target,
            raw_json,
            deferred_response,
            command_output,
            internal_dispatch_call_id,
        );
        let replaced_inspector_dispatch = self
            ._context_host
            .borrow_mut()
            .replace_active_inspector_dispatch(previous_inspector_dispatch);
        assert!(
            replaced_inspector_dispatch,
            "the V8 Inspector dispatch scope must remain active for the complete dispatch"
        );
        let replaced_runtime_command_cause = self
            ._context_host
            .borrow_mut()
            .replace_active_runtime_command_cause(previous_runtime_command_cause);
        assert_eq!(
            replaced_runtime_command_cause, runtime_command_cause,
            "the exact Runtime command output scope must remain active for the complete V8 dispatch"
        );
        if file_prompt_handler.is_some() {
            self._context_host
                .borrow_mut()
                .end_webdriver_bidi_file_prompt_handler();
        }
        if user_gesture {
            self._context_host
                .borrow_mut()
                .end_protocol_user_gesture_activation();
        }
        result
    }

    fn dispatch_inspector_protocol_message_with_current_activation(
        &mut self,
        target: PageInspectorSessionTarget<'_>,
        raw_json: &str,
        deferred_response: Option<RendererRuntimeInspectorResponseSender>,
        command_output: Option<crate::runtime::RendererRuntimeCommandOutputRecorder>,
        internal_dispatch_call_id: Option<i32>,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        let timing_started = moli_trace::cdp_nav_timing_enabled().then(Instant::now);
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        let inspector_window_dispatch_scope =
            runtime_protocol_message_window_dispatch_target(raw_json)
                .and_then(|target| self.inspector_window_dispatch_scope_for_target(target));
        let runtime_binding_replay_context_ptrs = self.runtime_binding_replay_context_ptrs();
        let renderer_document_isolate = self.renderer_document_isolate.clone();
        let page_inspector = &mut self.page_inspector;
        let root_frame_id = self.root_frame_id.clone();
        let deferred_call_id = deferred_response
            .as_ref()
            .map(|callback| callback.call_id());
        let captures_command_output = command_output.is_some();
        let messages = renderer_document_isolate
            .with_entered_renderer_document_isolate_and_inspector_mut(|isolate, inspector| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let previous_window_dispatch_scope = inspector_window_dispatch_scope
                    .map(|owner| enter_inspector_window_dispatch_scope(scope, owner));
                let messages = page_inspector.with_session_and_outbound(
                    inspector,
                    target,
                    |session, outbound, runtime_bindings_to_replay| -> Result<Vec<Value>> {
                        let replay_global_snapshots =
                            capture_runtime_binding_replay_global_snapshots(
                                scope,
                                &runtime_binding_replay_context_ptrs,
                                &runtime_bindings_to_replay,
                            );
                        for (index, binding) in runtime_bindings_to_replay.iter().enumerate() {
                            let (replay_call_id, replay_request) =
                                runtime_binding_replay_request_json(binding, index)?;
                            let replay_snap = outbound.len();
                            {
                                let _internal_response_capture =
                                    outbound.capture_internal_dispatch_response(replay_call_id);
                                let _dispatch_response_capture =
                                    outbound.capture_dispatch_responses();
                                with_scoped_inspector_microtasks(scope, || {
                                    session.dispatch_protocol_message(
                                        v8::inspector::StringView::from(replay_request.as_bytes()),
                                    );
                                });
                            }
                            outbound.discard_messages_after(replay_snap);
                        }
                        restore_runtime_binding_replay_global_snapshots(
                            scope,
                            replay_global_snapshots,
                        );
                        let snap = outbound.len();
                        if let Some(command_output) = command_output.clone() {
                            outbound.begin_runtime_command_output(command_output);
                        }
                        if let Some(callback) = deferred_response {
                            outbound.register_frontend_response_callback(callback);
                        }
                        let _internal_response_capture = internal_dispatch_call_id
                            .map(|call_id| outbound.capture_internal_dispatch_response(call_id));
                        // Active dispatch responses are returned directly from this call. Deferred
                        // awaitPromise responses with a registered callback are delivered to that
                        // callback even if they settle in this same owner turn.
                        let dispatch_response_capture = outbound.capture_dispatch_responses();
                        let dispatch_started = timing_started.map(|_| Instant::now());
                        if let Err(error) = with_scoped_inspector_microtasks(scope, || {
                            dispatch_with_runtime_defaults(session, raw_json, &outbound)
                        }) {
                            outbound.discard_messages_after(snap);
                            return Err(anyhow::Error::msg(error));
                        }
                        if let (Some(total_started), Some(started)) =
                            (timing_started, dispatch_started)
                        {
                            tracing::info!(
                                target: "moli_cdp_nav_timing",
                                stage = "renderer_inspector_dispatch_protocol_message_done",
                                phase_ms = started.elapsed().as_millis(),
                                elapsed_ms = total_started.elapsed().as_millis(),
                            );
                        }
                        if runtime_protocol_message_runs_embedder_microtask_checkpoint(raw_json) {
                            let microtask_started = timing_started.map(|_| Instant::now());
                            Self::perform_microtask_checkpoints(scope, None)?;
                            if let (Some(total_started), Some(started)) =
                                (timing_started, microtask_started)
                            {
                                tracing::info!(
                                    target: "moli_cdp_nav_timing",
                                    stage = "renderer_inspector_microtask_checkpoint_done",
                                    phase_ms = started.elapsed().as_millis(),
                                    elapsed_ms = total_started.elapsed().as_millis(),
                                );
                            }
                        }
                        drop(dispatch_response_capture);
                        Ok(outbound.take_messages_after(snap))
                    },
                );
                if let (Some(owner), Some(previous)) = (
                    inspector_window_dispatch_scope,
                    previous_window_dispatch_scope.as_ref(),
                ) {
                    restore_inspector_window_dispatch_scope(scope, owner, previous);
                }
                let messages = messages?;
                page_inspector.record_execution_context_state(&messages, root_frame_id.as_deref());
                Ok(messages)
            });
        let messages = match messages {
            Ok(messages) => messages,
            Err(error) => {
                if captures_command_output
                    && let Some(inspector_session_id) = target.frontend_session_id()
                {
                    page_inspector.end_runtime_command_output_for_session(inspector_session_id);
                }
                if let Some(call_id) = deferred_call_id {
                    match target {
                        PageInspectorSessionTarget::Frontend(inspector_session_id) => {
                            page_inspector.cancel_response_callback_for_session(
                                inspector_session_id,
                                call_id,
                            );
                        }
                        PageInspectorSessionTarget::InternalRuntimeEvaluate => {
                            page_inspector.cancel_internal_runtime_evaluate_response(call_id);
                        }
                    }
                }
                return Err(error);
            }
        };
        if let Some(started) = timing_started {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "renderer_inspector_entered_scope_done",
                messages = messages.len(),
                elapsed_ms = started.elapsed().as_millis(),
            );
        }
        let record_sync_started = timing_started.map(|_| Instant::now());
        self.sync_child_browsing_context_records();
        if let (Some(total_started), Some(started)) = (timing_started, record_sync_started) {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "renderer_inspector_child_context_sync_done",
                phase_ms = started.elapsed().as_millis(),
                elapsed_ms = total_started.elapsed().as_millis(),
            );
        }
        self.finish_runtime_turn_with_style_drain(
            crate::style_engine::StyleInvalidationTurnExitBoundary::RuntimeEvaluate,
            (),
        );
        self.page_isolated_world_contexts
            .record_inspector_context_state(&messages, self.root_frame_id.as_deref());
        if let Some(started) = timing_started {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "renderer_inspector_dispatch_done",
                messages = messages.len(),
                elapsed_ms = started.elapsed().as_millis(),
            );
        }
        Ok(self.runtime_inspector_messages_from_v8_messages(messages))
    }
}

impl ScriptVmPageRealmBootstrap {
    fn new_from_dom_host(
        dom_host: DomHost,
        bypass_content_security_policy: bool,
        page_task_tx: RuntimePageTaskSender,
        page_task_parser_boundary_injection_tx: tokio::sync::mpsc::UnboundedSender<PageTask>,
        resource_completion_tx: RendererResourceCompletionSender,
        initial_document_loader_bootstrap: crate::network::context::DocumentResourceLoaderBootstrap,
        browser_context_runtime: RendererBrowserContextRuntime,
        javascript_dialog_runtime: crate::runtime::RendererJavaScriptDialogRuntime,
        renderer_document_isolate_bootstrap: RendererDocumentIsolateBootstrap,
        runtime_inspector_session_restore_snapshots:
            &[crate::runtime::RendererInspectorSessionRestoreSnapshot],
        backend_node_registry: SharedRendererBackendNodeRegistry,
        root_frame_id: Option<String>,
        main_document_commit: Option<crate::runtime::RendererMainDocumentCommit>,
        top_level_storage_key: Option<moli_storage_key::MoliStorageKey>,
        reserved_service_worker_client_id: Option<
            crate::service_worker_runtime::ServiceWorkerClientId,
        >,
    ) -> std::result::Result<Self, ScriptVmBootstrapError> {
        let document_handle = dom_host.document_handle();
        let document_url = dom_host
            .dom()
            .final_url()
            .expect("parsed native dom must retain a document url")
            .clone();
        let document_base_url = dom_host
            .document_base_url_for_handle(document_handle)
            .unwrap_or_else(|| document_url.clone());
        let mut frame_owner_store = FrameOwnerStore::default();
        frame_owner_store.ensure_main_frame(
            document_handle,
            document_url.clone(),
            document_base_url,
            moli_url::origin_ascii_serialization(&document_url),
            crate::document_runtime::DocumentPolicyContainer::default(),
            crate::types::SubresourcePolicyContext::default(),
            None,
        );
        let main_document_owner = frame_owner_store
            .current_main_document_task_owner()
            .expect("main frame admission must produce a Document owner");
        let post_domcontentloaded_page_task_tx = page_task_tx.page_task_sender();
        let page_runtime_wake_tx = page_task_tx.page_runtime_wake_sender();
        let top_level_navigation_handoff_tx = page_task_tx.top_level_navigation_handoff_sender();
        let service_worker_task_tx = page_task_tx.service_worker_task_sender();
        let stylesheet_task_sender = page_task_tx.stylesheet_task_sender();
        let main_parser_continuation_sender = page_task_tx.main_parser_continuation_sender();
        let resource_owner_id = crate::resource_owner::ResourceOwnerId::new();
        let author_styles_disabled = initial_document_loader_bootstrap.author_styles_disabled();
        let mut document_runtime = Box::new(DocumentRuntime::from_main_frame_dom_host(
            dom_host,
            main_document_owner,
            Some(post_domcontentloaded_page_task_tx.clone()),
            page_task_parser_boundary_injection_tx,
            stylesheet_task_sender,
            main_parser_continuation_sender,
        ));
        document_runtime.set_author_styles_disabled(author_styles_disabled);
        document_runtime.set_bypass_content_security_policy(bypass_content_security_policy);
        let (page_context_cancel_tx, page_context_cancel_rx) =
            renderer_page_context_cancel_channel();

        let RendererDocumentIsolateBootstrap {
            renderer_document_isolate,
            bridge_bindings,
            renderer_document_isolate_teardown,
            page_inspector,
            renderer_page_script_environment,
            reuse_main_window_proxy,
        } = renderer_document_isolate_bootstrap;
        renderer_document_isolate.with_renderer_document_isolate_and_inspector_mut(|_, backend| {
            page_inspector
                .reattach_v8_sessions(backend, runtime_inspector_session_restore_snapshots);
        });
        if let (Some(environment), Some(commit)) = (
            renderer_page_script_environment.as_ref(),
            main_document_commit,
        ) {
            // V8 session reattachment above has already appended
            // executionContextsCleared. The default world has not been
            // created yet, so this exact append point gives the Page FIFO the
            // same reset -> frame commit -> context-created order as Blink.
            environment.output_journal().append(
                crate::runtime::PendingRendererOutputRecord::observation(
                    None,
                    crate::runtime::RendererProtocolObservation::MainDocumentCommit(commit),
                ),
            );
        }
        let dom_debugger_pause_scheduler = page_inspector.dom_debugger_pause_scheduler();

        let context_host = Rc::new(RefCell::new(JsContextHost::new(
            document_runtime.as_mut(),
            frame_owner_store,
            bridge_bindings,
            backend_node_registry,
            dom_debugger_pause_scheduler,
            resource_completion_tx,
            top_level_navigation_handoff_tx,
            service_worker_task_tx,
            browser_context_runtime,
            javascript_dialog_runtime,
            page_context_cancel_rx,
            top_level_storage_key,
            reserved_service_worker_client_id,
        )));
        context_host
            .borrow_mut()
            .bind_deferred_context_host_release_queue(
                renderer_document_isolate.deferred_context_host_release_queue(),
            );
        let main_document_owner = context_host
            .borrow()
            .current_main_document_task_owner()
            .expect("main Document owner must exist after native host construction");
        let initial_document_context = {
            let context_host = context_host.borrow();
            let document_url = context_host.document_url().clone();
            let document_handle = context_host.document_handle();
            crate::network::context::DocumentFetchContext::new(
                crate::native_bridge::WindowDocumentOwner::Frame(main_document_owner),
                document_url.clone(),
                context_host.document_base_url_for_handle(document_handle),
                moli_url::origin_ascii_serialization(&document_url),
            )
        };
        let initial_document_loader =
            initial_document_loader_bootstrap.commit(initial_document_context);
        context_host
            .borrow_mut()
            .register_main_document_resource_loader(&initial_document_loader);
        document_runtime.set_cookie_store(initial_document_loader.request_client().cookie_store());
        assert_eq!(
            document_runtime.has_main_document_runtime_route(),
            page_runtime_wake_tx.has_main_document_runtime_route(),
            "main Document runtime construction must match the PageVm route capability"
        );
        {
            let context_host = context_host.borrow();
            document_runtime.set_service_worker_connected_link_context(
                context_host.browser_context_runtime(),
                context_host.service_worker_client_id(),
            );
        }
        let promise_reject_dispatch = promise_reject_dispatch_slot(context_host.clone());
        let prebootstrapped_child_default_contexts = Rc::new(RefCell::new(HashMap::new()));
        context_host
            .borrow_mut()
            .install_child_default_context_bootstrap(
                Rc::downgrade(&context_host),
                Rc::downgrade(&prebootstrapped_child_default_contexts),
                resource_owner_id,
                promise_reject_dispatch.clone(),
            );

        Ok(Self {
            resource_owner_id,
            promise_reject_dispatch,
            page_inspector,
            renderer_document_isolate,
            renderer_document_isolate_teardown,
            document_runtime,
            root_frame_id,
            context_host,
            prebootstrapped_child_default_contexts,
            page_context_cancel_tx,
            post_domcontentloaded_page_task_tx,
            page_runtime_wake_tx,
            storage_bucket_store: crate::context_bootstrap::new_shared_storage_bucket_store(),
            renderer_page_script_environment,
            reuse_main_window_proxy,
        })
    }

    fn bootstrap_default_world(
        self,
    ) -> std::result::Result<ScriptVmDefaultWorldBootstrap, ScriptVmBootstrapError> {
        let ScriptVmPageRealmBootstrap {
            resource_owner_id,
            promise_reject_dispatch,
            mut page_inspector,
            renderer_document_isolate,
            renderer_document_isolate_teardown,
            document_runtime,
            root_frame_id,
            context_host,
            prebootstrapped_child_default_contexts,
            page_context_cancel_tx,
            post_domcontentloaded_page_task_tx,
            page_runtime_wake_tx,
            storage_bucket_store,
            renderer_page_script_environment,
            reuse_main_window_proxy,
        } = self;
        let context_bootstrap = match renderer_document_isolate
            .with_entered_renderer_document_isolate_and_bootstrap(|isolate, isolate_bootstrap| {
                ScriptVmContextBootstrap::new_main_default(
                    isolate,
                    isolate_bootstrap,
                    context_host.clone(),
                    resource_owner_id,
                    &promise_reject_dispatch,
                    None,
                    Some(storage_bucket_store.clone()),
                    renderer_page_script_environment.clone(),
                    reuse_main_window_proxy,
                )
            }) {
            Ok(context) => context,
            Err(error) => {
                return Err(Box::new((
                    error,
                    recover_bootstrap_dom_host_from_holder(
                        renderer_document_isolate,
                        renderer_document_isolate_teardown,
                        context_host,
                        document_runtime,
                    ),
                )));
            }
        };
        let runtime_observable_context_token = context_bootstrap.runtime_observable_context_token;
        let (context, bridge_ref) = context_bootstrap.into_context_and_bridge_ref();
        if let Err(error) =
            renderer_document_isolate.with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let local_context = v8::Local::new(scope, &context);
                context_host
                    .borrow_mut()
                    .install_page_default_context(scope, local_context);
                let scope = &mut v8::ContextScope::new(scope, local_context);
                super::context_bootstrap::initialize_main_session_history(scope);
                Ok(())
            })
        {
            return Err(Box::new((
                error,
                recover_bootstrap_dom_host_from_holder(
                    renderer_document_isolate,
                    renderer_document_isolate_teardown,
                    context_host,
                    document_runtime,
                ),
            )));
        }
        let inspector_document_isolate = renderer_document_isolate.clone();
        let baseline_globals = match renderer_document_isolate
            .with_renderer_document_isolate_and_inspector_mut(|isolate, inspector| {
                ScriptVmDefaultWorldBootstrap::attach_context_and_capture_baseline_globals(
                    inspector_document_isolate,
                    isolate,
                    inspector,
                    &mut page_inspector,
                    &context,
                    document_runtime.document_url(),
                    root_frame_id.as_deref(),
                )
            }) {
            Ok(baseline_globals) => baseline_globals,
            Err(error) => {
                drop(page_inspector);
                return Err(Box::new((
                    error,
                    recover_bootstrap_dom_host_from_holder(
                        renderer_document_isolate,
                        renderer_document_isolate_teardown,
                        context_host,
                        document_runtime,
                    ),
                )));
            }
        };
        if let Err(error) = register_main_window_execution_context_for_bootstrap(
            &renderer_document_isolate,
            &context_host,
            &context,
        ) {
            drop(page_inspector);
            drop(bridge_ref);
            drop(context);
            drop(promise_reject_dispatch);
            return Err(Box::new((
                error,
                recover_bootstrap_dom_host_from_holder(
                    renderer_document_isolate,
                    renderer_document_isolate_teardown,
                    context_host,
                    document_runtime,
                ),
            )));
        }
        Ok(ScriptVmDefaultWorldBootstrap {
            resource_owner_id,
            promise_reject_dispatch,
            page_inspector,
            renderer_document_isolate,
            renderer_document_isolate_teardown,
            page_default_context: context,
            bridge_ref,
            runtime_observable_context_token,
            baseline_globals,
            document_runtime,
            root_frame_id,
            context_host,
            prebootstrapped_child_default_contexts,
            page_context_cancel_tx,
            post_domcontentloaded_page_task_tx,
            page_runtime_wake_tx,
            storage_bucket_store,
            renderer_page_script_environment,
        })
    }
}

impl ScriptVmDefaultWorldBootstrap {
    #[cfg(test)]
    fn standalone_from_dom_host_with_resource_completion_sender_and_browser_context_runtime_for_test_with_current_runtime(
        bootstrap_dom_host: DomHost,
        page_task_tx: RuntimePageTaskSender,
        page_task_parser_boundary_injection_tx: tokio::sync::mpsc::UnboundedSender<PageTask>,
        resource_completion_tx: RendererResourceCompletionSender,
        initial_document_loader_bootstrap: crate::network::context::DocumentResourceLoaderBootstrap,
        browser_context_runtime: RendererBrowserContextRuntime,
    ) -> std::result::Result<Self, ScriptVmBootstrapError> {
        let renderer_document_isolate_bootstrap =
            match RendererDocumentIsolateHandle::new_standalone_without_owner_reservation_for_test(
                page_task_tx.v8_foreground_task_sender(),
            ) {
                Ok(bootstrap) => bootstrap,
                Err(error) => return Err(Box::new((error, bootstrap_dom_host))),
            };
        Self::from_dom_host_with_resource_completion_sender_browser_context_runtime_and_document_isolate(
            bootstrap_dom_host,
            false,
            page_task_tx,
            page_task_parser_boundary_injection_tx,
            resource_completion_tx,
            initial_document_loader_bootstrap,
            browser_context_runtime,
            crate::runtime::RendererJavaScriptDialogRuntime::default(),
            renderer_document_isolate_bootstrap,
            &[],
            crate::runtime::new_shared_renderer_backend_node_registry(),
            None,
            None,
            None,
            None,
        )
    }

    pub(super) fn from_dom_host_with_resource_completion_sender_browser_context_runtime_and_document_isolate(
        bootstrap_dom_host: DomHost,
        bypass_content_security_policy: bool,
        page_task_tx: RuntimePageTaskSender,
        page_task_parser_boundary_injection_tx: tokio::sync::mpsc::UnboundedSender<PageTask>,
        resource_completion_tx: RendererResourceCompletionSender,
        initial_document_loader_bootstrap: crate::network::context::DocumentResourceLoaderBootstrap,
        browser_context_runtime: RendererBrowserContextRuntime,
        javascript_dialog_runtime: crate::runtime::RendererJavaScriptDialogRuntime,
        renderer_document_isolate_bootstrap: RendererDocumentIsolateBootstrap,
        runtime_inspector_session_restore_snapshots:
            &[crate::runtime::RendererInspectorSessionRestoreSnapshot],
        backend_node_registry: SharedRendererBackendNodeRegistry,
        root_frame_id: Option<String>,
        main_document_commit: Option<crate::runtime::RendererMainDocumentCommit>,
        top_level_storage_key: Option<moli_storage_key::MoliStorageKey>,
        reserved_service_worker_client_id: Option<
            crate::service_worker_runtime::ServiceWorkerClientId,
        >,
    ) -> std::result::Result<Self, ScriptVmBootstrapError> {
        ScriptVmPageRealmBootstrap::new_from_dom_host(
            bootstrap_dom_host,
            bypass_content_security_policy,
            page_task_tx,
            page_task_parser_boundary_injection_tx,
            resource_completion_tx,
            initial_document_loader_bootstrap,
            browser_context_runtime,
            javascript_dialog_runtime,
            renderer_document_isolate_bootstrap,
            runtime_inspector_session_restore_snapshots,
            backend_node_registry,
            root_frame_id,
            main_document_commit,
            top_level_storage_key,
            reserved_service_worker_client_id,
        )?
        .bootstrap_default_world()
    }

    fn attach_context_and_capture_baseline_globals(
        renderer_document_isolate: RendererDocumentIsolateHandle,
        isolate: &mut v8::OwnedIsolate,
        inspector: &mut RendererInspectorIsolateBackend,
        page_inspector: &mut DocumentInspectorBinding,
        context: &v8::Global<v8::Context>,
        document_url: &Url,
        root_frame_id: Option<&str>,
    ) -> Result<ScriptGlobalsBaseline> {
        let scope = pin!(v8::HandleScope::new(isolate));
        let scope = &mut scope.init();
        let local_context = v8::Local::new(scope, context);
        let default_context = v8::Global::new(scope.as_ref(), local_context);
        let registered_context = v8::Global::new(scope.as_ref(), local_context);
        let scope = &mut v8::ContextScope::new(scope, local_context);
        page_inspector.attach_context(
            renderer_document_isolate,
            inspector,
            local_context,
            default_context,
            registered_context,
            document_url,
            root_frame_id,
        );

        #[cfg(not(any(test, feature = "test-support")))]
        {
            let _ = scope;
            Ok(ScriptGlobalsBaseline)
        }

        #[cfg(any(test, feature = "test-support"))]
        {
            let baseline_source = v8_string(
                scope,
                "JSON.stringify(Object.getOwnPropertyNames(globalThis))",
            )
            .ok_or_else(|| anyhow!("failed to allocate v8 baseline snapshot source string"))?;
            let baseline_script = v8::Script::compile(scope, baseline_source, None)
                .ok_or_else(|| anyhow!("v8 failed to compile baseline snapshot script"))?;
            let baseline_value =
                crate::script_execution::execute_compiled_script(scope, baseline_script)
                    .ok_or_else(|| anyhow!("v8 failed to execute baseline snapshot script"))?;
            let baseline_json = baseline_value
                .to_string(scope)
                .ok_or_else(|| anyhow!("v8 baseline snapshot did not return a string"))?
                .to_rust_string_lossy(scope);

            serde_json::from_str(&baseline_json)
                .context("failed to deserialize baseline v8 globals")
        }
    }

    pub fn finish(self) -> std::result::Result<ScriptVm, ScriptVmBootstrapError> {
        let Self {
            renderer_document_isolate,
            renderer_document_isolate_teardown,
            renderer_page_script_environment,
            page_default_context: context,
            bridge_ref,
            runtime_observable_context_token,
            baseline_globals,
            document_runtime,
            context_host,
            prebootstrapped_child_default_contexts,
            page_context_cancel_tx,
            post_domcontentloaded_page_task_tx,
            page_runtime_wake_tx,
            resource_owner_id,
            promise_reject_dispatch,
            page_inspector,
            storage_bucket_store,
            root_frame_id,
        } = self;
        let vm = ScriptVm {
            resource_owner_id,
            page_inspector,
            renderer_document_isolate,
            renderer_document_isolate_teardown,
            renderer_page_script_environment,
            page_default_context: context,
            page_default_bridge_ref: Some(bridge_ref),
            page_isolated_world_contexts: PageIsolatedWorldRegistry::new(),
            child_frame_realm_store: child_frame_realm::ChildFrameRealmStore::default(),
            prebootstrapped_child_default_contexts,
            child_document_modulator_store: ChildDocumentModulatorStore::default(),
            page_default_runtime_observable_context_token: runtime_observable_context_token,
            root_frame_id,
            baseline_globals,
            document_runtime: ScriptVmDocumentRuntimeOwner::new(document_runtime),
            _context_host: context_host,
            page_context_cancel_tx,
            post_domcontentloaded_page_task_tx,
            page_runtime_wake_tx,
            queued_main_document_runtime_continuation_owner: None,
            queued_main_document_module_continuation_owner: None,
            queued_main_document_parser_module_continuation_owner: None,
            script_execution_memory: ScriptExecutionMemoryCounters::default(),
            runtime_observable_source_queue: RendererRuntimeObservableSourceQueue::default(),
            pressed_mouse_buttons: 0,
            pending_mouse_press: None,
            hovered_mouse_handle: None,
            hovered_mouse_root_to_frame: moli_layout::LayoutTransform2D::IDENTITY,
            active_touch_pointer_handle: None,
            active_touch_pointer_handles: BTreeMap::new(),
            active_touch_event_handle: None,
            active_touch_point: None,
            active_touch_points: BTreeMap::new(),
            suppress_next_keypress_after_canceled_raw_keydown: false,
            suppress_compat_mouse_events: false,
            active_scrollbar_drag: None,
            active_drag_session: None,
            promise_reject_dispatch,
            next_internal_runtime_evaluate_call_id: 1,
            next_internal_frontend_inspector_call_id: -1,
            pending_internal_runtime_evaluates: HashMap::new(),
            indexed_db_manager: None,
            storage_bucket_store,
            app_manifest_cache: None,
            #[cfg(test)]
            test_next_timeout_failure: None,
            #[cfg(test)]
            _page_task_residence_for_executor_test: None,
        };
        vm._context_host
            .borrow_mut()
            .set_storage_bucket_store(vm.storage_bucket_store.clone());
        if let Some(environment) = &vm.renderer_page_script_environment {
            vm._context_host
                .borrow_mut()
                .bind_output_journal(environment.output_journal());
        }
        vm._context_host.borrow_mut().publish_document_host();
        vm.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                crate::util::retain_context_host_for_document_realm(
                    v8::Local::new(scope, &vm.page_default_context),
                    vm._context_host.clone(),
                    vm.renderer_document_isolate
                        .deferred_context_host_release_queue(),
                );
                for child in vm.prebootstrapped_child_default_contexts.borrow().values() {
                    crate::util::retain_context_host_for_document_realm(
                        v8::Local::new(scope, &child.context),
                        vm._context_host.clone(),
                        vm.renderer_document_isolate
                            .deferred_context_host_release_queue(),
                    );
                }
            });
        Ok(vm)
    }
}

impl ScriptVm {
    pub(crate) fn current_main_document_resource_loader(&self) -> Option<DocumentResourceLoader> {
        self._context_host
            .borrow()
            .current_main_document_resource_loader()
    }

    #[cfg(test)]
    pub(crate) fn resource_completion_sender_for_test(
        &self,
    ) -> crate::page_task_queue::RendererResourceCompletionSender {
        self._context_host.borrow().resource_completion_sender()
    }

    #[cfg(test)]
    pub(crate) fn websocket_sender_for_test(
        &self,
    ) -> crate::page_task_queue::RendererPageWebSocketSender {
        self._context_host.borrow().page_websocket_sender().clone()
    }

    #[cfg(test)]
    pub(crate) fn worker_host_bridge_sender_for_test(
        &self,
    ) -> crate::page_task_queue::RendererWorkerHostBridgeEventSender {
        self._context_host
            .borrow()
            .page_worker_host_bridge_event_sender()
            .clone()
    }

    pub(crate) fn page_context_cancel_sender(&self) -> RendererPageContextCancelSender {
        self.page_context_cancel_tx.clone()
    }

    pub(crate) fn cancel_page_context(&self, reason: RendererPageContextCancelReason) {
        self.page_context_cancel_tx.cancel(reason);
    }

    pub(super) fn register_internal_node_reference(&mut self, handle: DomHandle) -> Option<u64> {
        self._context_host
            .borrow_mut()
            .register_internal_node_reference(handle)
    }

    pub(super) fn discard_internal_node_reference(&mut self, token: u64) {
        self._context_host
            .borrow_mut()
            .discard_internal_node_reference(token);
    }

    pub(super) fn close_page_context_resources_for_context_teardown(&mut self) {
        self.clear_context_wrapper_caches_for_context_teardown();
        clear_promise_rejection_dispatch_state(&self.promise_reject_dispatch);
        self._context_host
            .borrow_mut()
            .close_page_context_resources_for_teardown();
    }

    fn clear_context_wrapper_caches_for_context_teardown(&mut self) {
        let mut context_ptrs: Vec<*const v8::Global<v8::Context>> = Vec::with_capacity(
            1 + self.page_isolated_world_contexts.len() + self.child_frame_realm_store.len(),
        );
        context_ptrs.push(&self.page_default_context as *const _);
        context_ptrs.extend(
            self.page_isolated_world_contexts
                .contexts()
                .map(|world| &world.context as *const _),
        );
        context_ptrs.extend(
            self.child_frame_realm_store
                .values()
                .map(|world| &world.context as *const _),
        );

        for (index, context_ptr) in context_ptrs.into_iter().enumerate() {
            self.clear_context_wrapper_cache_for_context_ptr(context_ptr, index == 0);
        }
    }

    fn clear_context_wrapper_cache_for_context_ptr(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        include_shared_default_world: bool,
    ) {
        let _ = self
            .renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                crate::native_bridge::clear_context_wrapper_cache_for_teardown(
                    scope,
                    include_shared_default_world,
                );
                Ok(())
            });
    }

    pub(super) fn detach_default_inspector_context_for_context_teardown(&mut self) {
        self.renderer_document_isolate
            .with_renderer_document_isolate_and_inspector_mut(|_isolate, inspector| {
                self.page_inspector
                    .detach_default_context_from_backend_if_same(inspector);
            });
        // `reset_context_group` synchronously emits the old document's
        // Runtime.executionContextsCleared notification. Preserve that event,
        // then sever this backend's route before any later teardown work can
        // target a replacement PageVM that reuses the target registry.
        self.page_inspector
            .deactivate_page_vm_binding_for_teardown();
    }

    pub(super) fn detach_main_window_proxy_for_navigation_commit(
        &mut self,
        page_id: u64,
    ) -> Result<()> {
        let environment = self
            .renderer_page_script_environment
            .as_ref()
            .ok_or_else(|| anyhow!("main navigation requires a page script environment"))?
            .clone();
        if environment.page_id() != page_id {
            return Err(anyhow!(
                "main navigation crossed page script environment ownership"
            ));
        }
        let isolate_identity_key = self.renderer_document_isolate.identity_key();
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let global_proxy = context.global(scope);
                let is_stable_proxy = environment.with_main_window_proxy(|stable_proxy| {
                    v8::Local::new(scope, stable_proxy).strict_equals(global_proxy.into())
                })?;
                if !is_stable_proxy {
                    return Err(anyhow!(
                        "main context global does not match its page-owned WindowProxy"
                    ));
                }
                context.detach_global();
                Ok(())
            })?;
        tracing::debug!(
            page_id,
            isolate_identity_key,
            "detached committed main WindowProxy for replacement context"
        );
        Ok(())
    }

    pub(super) fn sync_live_document_style_sources_if_pending(&mut self) {
        if self
            .document_runtime
            .take_style_source_document_sync_pending()
        {
            self.sync_live_document_style_sources();
        }
    }

    pub(super) fn requires_deferred_lifo_drop(&self) -> bool {
        self.renderer_document_isolate_teardown
            .requires_deferred_lifo_script_vm_drop()
    }

    pub(super) fn unregister_document_isolate_platform_for_context_teardown(&self) {
        self.renderer_document_isolate_teardown
            .unregister_platform_on_context_teardown(&self.renderer_document_isolate);
    }
}

fn map_child_viewport_point_to_parent_content(
    point: moli_layout::LayoutPoint,
    child_viewport: moli_layout::LayoutViewport,
    parent_content: moli_layout::LayoutQuad,
) -> moli_layout::LayoutPoint {
    let [origin, x_corner, _, y_corner] = parent_content.points;
    let u = if child_viewport.css_width == 0 {
        0.0
    } else {
        f64::from(point.x) / f64::from(child_viewport.css_width)
    };
    let v = if child_viewport.css_height == 0 {
        0.0
    } else {
        f64::from(point.y) / f64::from(child_viewport.css_height)
    };
    moli_layout::LayoutPoint::new(
        (f64::from(origin.x)
            + f64::from(x_corner.x - origin.x) * u
            + f64::from(y_corner.x - origin.x) * v) as f32,
        (f64::from(origin.y)
            + f64::from(x_corner.y - origin.y) * u
            + f64::from(y_corner.y - origin.y) * v) as f32,
    )
}

fn module_script_source_for_execution(
    script_url: &Url,
    source: &str,
    source_bytes: Option<&[u8]>,
) -> std::result::Result<ModuleSource, String> {
    if script_url.path().to_ascii_lowercase().ends_with(".wasm") {
        let bytes = source_bytes.ok_or_else(|| {
            format!("WebAssembly module script `{script_url}` did not retain binary source")
        })?;
        return Ok(ModuleSource::binary(bytes.to_vec()));
    }
    Ok(ModuleSource::text(source.to_owned()))
}

pub(crate) fn prepared_script_uses_external_module_graph(script: &PreparedScript) -> bool {
    script.kind == ScriptKind::Module
        && script.source_kind == ScriptSourceKind::External
        && script.url.scheme() != "data"
        && matches!(script.source, crate::planning::ScriptSource::External)
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum SerializedJsValue {
    Undefined,
    Null,
    Boolean { value: bool },
    Number { value: f64 },
    String { value: String },
    Unsupported { value: String },
}

#[cfg(any(test, feature = "test-support"))]
impl SerializedJsValue {
    fn into_snapshot(self) -> JsValueSnapshot {
        match self {
            Self::Undefined => JsValueSnapshot::Undefined,
            Self::Null => JsValueSnapshot::Null,
            Self::Boolean { value } => JsValueSnapshot::Bool(value),
            Self::Number { value } => JsValueSnapshot::Number(value),
            Self::String { value } => JsValueSnapshot::String(value),
            Self::Unsupported { value } => JsValueSnapshot::Unsupported(value),
        }
    }
}

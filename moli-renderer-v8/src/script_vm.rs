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
    context_bootstrap::{
        dispatch_media_query_list_change_events as dispatch_media_query_list_change_events_for_scope,
        set_date_locale_override_for_current_context,
        set_date_timezone_override_for_current_context, set_window_navigator_identity,
        sync_global_location_runtime_state,
    },
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
mod rendering_update;
mod runtime_bindings;
mod runtime_script_continuation;
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
    RendererDocumentIsolateBootstrap, RendererDocumentIsolateHandle,
    RendererDocumentIsolateReservationAccounting, RendererPageScriptEnvironment,
    ScriptVmDefaultWorldBootstrap, renderer_document_isolate_accounting_diagnostics,
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

pub(super) struct ScriptVm {
    resource_owner_id: crate::resource_owner::ResourceOwnerId,
    /// Page/target-facing inspector state. This must drop before the renderer
    /// document isolate handle because the V8 inspector session touches the
    /// isolate-level backend while being destroyed.
    page_inspector: DocumentInspectorBinding,
    /// Handle to renderer-owner document isolate-level V8 state. Multiple page
    /// facades can share the holder while keeping page/context state here.
    renderer_document_isolate: RendererDocumentIsolateHandle,
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
    pub(super) document_runtime: Box<DocumentRuntime>,
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
}

impl moli_layout::GeometryProvider for ScriptVm {
    type NodeId = DomHandle;

    fn answer(
        &mut self,
        reason: moli_layout::LayoutFlushReason,
        viewport: moli_layout::LayoutViewport,
        queries: &moli_layout::LayoutQueryBatch<Self::NodeId>,
    ) -> Result<moli_layout::LayoutAnswers<Self::NodeId>, moli_layout::LayoutError> {
        let needs_refresh = {
            let context_host = self._context_host.borrow();
            !context_host.can_answer_layout_from_snapshot(context_host.document_handle())
        };
        if needs_refresh {
            self.reconcile_document_web_fonts_for_layout();
        }
        self._context_host
            .borrow_mut()
            .answer(reason, viewport, queries)
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
                        let Ok(unwrapped) = session.unwrap_object(
                            scope,
                            v8::inspector::StringView::from(object_id.as_bytes()),
                        ) else {
                            return Ok(None);
                        };
                        let Ok(object) = v8::Local::<v8::Object>::try_from(unwrapped.value) else {
                            return Ok(None);
                        };
                        let scope = &mut v8::ContextScope::new(scope, unwrapped.context);
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
        host.dom_host().outer_html_with_shadow_roots(
            handle,
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
                                    session.dispatch_protocol_message(
                                        v8::inspector::StringView::from(raw_json.as_bytes()),
                                    );
                                });
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
                        with_scoped_inspector_microtasks(scope, || {
                            session.dispatch_protocol_message(v8::inspector::StringView::from(
                                raw_json.as_bytes(),
                            ));
                        });
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
        let mut document_runtime = Box::new(DocumentRuntime::from_main_frame_dom_host(
            dom_host,
            main_document_owner,
            Some(post_domcontentloaded_page_task_tx.clone()),
            page_task_parser_boundary_injection_tx,
            stylesheet_task_sender,
            main_parser_continuation_sender,
        ));
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
            let baseline_value = baseline_script
                .run(scope)
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
            document_runtime,
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

    pub(crate) fn sync_live_document_style_sources(&mut self) {
        let document = self.document_runtime.document_handle();
        self._context_host
            .borrow_mut()
            .sync_owner_style_sheet_texts_for_document_tree_scopes(document);
    }

    pub(super) fn computed_style_property_values_for_document_snapshot(
        &self,
        handle: DomHandle,
        properties: &[String],
    ) -> Option<Vec<String>> {
        crate::native_bridge::element::computed_style_property_values_for_document_snapshot(
            &self._context_host.borrow(),
            handle,
            properties,
        )
    }

    #[cfg(test)]
    pub(super) fn screenshot_layout_snapshot(
        &mut self,
        viewport: moli_layout::PaintViewport,
    ) -> anyhow::Result<Option<moli_layout::PaintSnapshot>> {
        self.paint_layout_snapshot(viewport, moli_layout::LayoutFlushReason::Screenshot)
    }

    #[cfg(test)]
    pub(crate) fn refresh_layout_snapshot_for_test(
        &mut self,
        viewport: moli_layout::LayoutViewport,
    ) -> Result<bool, moli_layout::LayoutError> {
        self.with_fresh_layout_pass(
            moli_layout::LayoutPassRequest::new(viewport, moli_layout::LayoutFlushReason::Test),
            |_| Ok(()),
        )
        .map(|result| result.is_some())
    }

    #[cfg(test)]
    pub(super) fn paint_layout_snapshot(
        &mut self,
        viewport: moli_layout::PaintViewport,
        reason: moli_layout::LayoutFlushReason,
    ) -> anyhow::Result<Option<moli_layout::PaintSnapshot>> {
        self.paint_layout_snapshot_with_capture(
            viewport,
            reason,
            moli_layout::PaintCaptureRequest::viewport(),
        )
    }

    pub(super) fn paint_layout_snapshot_with_capture(
        &mut self,
        viewport: moli_layout::PaintViewport,
        reason: moli_layout::LayoutFlushReason,
        capture: moli_layout::PaintCaptureRequest,
    ) -> anyhow::Result<Option<moli_layout::PaintSnapshot>> {
        self.with_fresh_layout_pass(
            moli_layout::LayoutPassRequest::with_capture(viewport, reason, capture),
            moli_layout::LayoutPassResult::take_paint_snapshot,
        )
        .map_err(anyhow::Error::new)
    }

    pub(super) fn visual_state_token(
        &self,
        document: crate::runtime::RendererDocumentLifecycleIdentity,
        viewport: moli_layout::PaintViewport,
    ) -> crate::runtime::RendererVisualStateToken {
        self._context_host
            .borrow()
            .visual_state_token(document, viewport)
    }

    #[cfg(test)]
    pub(crate) fn visual_resource_generation_handle_for_test(
        &self,
    ) -> crate::native_bridge::visual_resource_generation::VisualResourceGeneration {
        self._context_host
            .borrow()
            .visual_resource_generation_handle_for_test()
    }

    fn with_fresh_layout_pass<T>(
        &mut self,
        request: moli_layout::LayoutPassRequest,
        consume: impl FnOnce(
            &mut moli_layout::LayoutPassResult<DomHandle>,
        ) -> Result<T, moli_layout::LayoutError>,
    ) -> Result<Option<T>, moli_layout::LayoutError> {
        // Font-source reconciliation is a pre-pass lifecycle step. CSS image
        // URLs come back from the actual box-construction traversal below.
        // Once the guard is entered, layout performs no JS, event-loop,
        // observer, or resource completion work and owns no state beyond this
        // call.
        self.reconcile_document_web_fonts_for_layout();
        let requests_paint = request.requests_paint();
        let (document, result) = {
            let context_host = self._context_host.borrow();
            let document = context_host.document_handle();
            let result =
                context_host.with_fresh_layout_pass_for_document(document, request, |pass| {
                    let css_images = if requests_paint {
                        pass.css_image_references().to_vec()
                    } else {
                        Vec::new()
                    };
                    consume(pass).map(|value| (value, css_images))
                });
            (document, result)
        };
        let (result, css_images) = match result {
            Ok(Some((value, css_images))) => (Ok(Some(value)), css_images),
            Ok(None) => (Ok(None), Vec::new()),
            Err(error) => (Err(error), Vec::new()),
        };
        self.start_css_images_discovered_by_layout(css_images);
        if matches!(&result, Ok(Some(_)))
            && let Err(error) = self.with_default_context_scope(|scope, runtime_ptr| {
                crate::native_bridge::element::queue_revealed_lazy_image_loads(
                    scope,
                    runtime_ptr,
                    document,
                );
                Ok(())
            })
        {
            // Entering the already-owned default context is infallible for
            // this body-only operation. Keep a failed admission non-fatal to
            // the completed frame; the next refresh retries from its newer
            // sampled geometry.
            tracing::warn!(?error, "failed to admit lazy images after layout refresh");
        }
        result
    }

    #[cfg(test)]
    pub(crate) fn layout_pass_observability_for_test(
        &self,
    ) -> (
        bool,
        u64,
        std::time::Duration,
        Option<moli_layout::LayoutPassMetrics>,
    ) {
        self._context_host
            .borrow()
            .layout_pass_observability_for_test()
    }

    #[cfg(test)]
    pub(crate) fn css_image_resource_is_ready_for_test(&self, resolved_url: &str) -> bool {
        let host = self._context_host.borrow();
        host.ready_css_image_for_layout(host.document_handle(), resolved_url)
            .is_some()
    }

    #[cfg(test)]
    pub(crate) fn css_image_resource_observability_for_test(
        &self,
    ) -> (usize, usize, usize, usize, Vec<String>) {
        self._context_host
            .borrow()
            .css_image_resource_observability_for_test()
    }

    #[cfg(test)]
    pub(crate) fn css_image_completion_notify_for_test(
        &self,
    ) -> std::sync::Arc<tokio::sync::Notify> {
        self._context_host
            .borrow()
            .css_image_completion_notify_for_test()
    }

    #[cfg(test)]
    pub(crate) fn layout_snapshot_cache_observability_for_test(
        &self,
    ) -> (
        u64,
        u64,
        u64,
        Option<(DomHandle, moli_layout::LayoutTreeRetentionMetrics)>,
    ) {
        let observability = self
            ._context_host
            .borrow()
            .layout_snapshot_cache_observability_for_test();
        (
            observability.hits,
            observability.misses,
            observability.publishes,
            observability.cached,
        )
    }

    fn reconcile_document_web_fonts_for_layout(&mut self) {
        let font_fetch_enabled = self
            .document_runtime
            .current_document_resource_loader()
            .is_some_and(|loader| {
                loader
                    .request_client()
                    .optional_resource_fetch_enabled(crate::types::SubresourceResourceType::Font)
            });
        if !font_fetch_enabled {
            return;
        }
        let root = {
            let host = self._context_host.borrow();
            host.dom_host().document_element_handle()
        };
        let Some(root) = root else {
            return;
        };
        let Some(resources) = crate::layout_renderer::current_native_stylesheet_resources(
            &self._context_host.borrow(),
            root,
        ) else {
            return;
        };
        tracing::trace!(
            stylesheet_import_dependency_count = resources.imports().len(),
            stylesheet_web_font_dependency_count = resources.web_fonts().len(),
            "observed typed stylesheet resource manifest"
        );
        let generation = resources.generation();
        if self
            ._context_host
            .borrow()
            .document_web_font_resources_are_current(generation)
        {
            return;
        }
        let resources = resources.web_fonts().to_vec();
        self._context_host
            .borrow()
            .retain_document_web_font_slots(resources.iter());
        if resources.is_empty() {
            self._context_host
                .borrow()
                .publish_document_web_font_resource_generation(generation);
            return;
        }
        let resource_count = resources.len();
        let bound = {
            let mut host = self._context_host.borrow_mut();
            let bound = resources
                .into_iter()
                .filter_map(|resource| {
                    host.accept_current_main_stylesheet_subresource_load_delay()
                        .map(|binding| (binding, resource))
                })
                .collect::<Vec<_>>();
            if bound.len() == resource_count {
                host.publish_document_web_font_resource_generation(generation);
            }
            bound
        };
        self.start_stylesheet_subresource_fetches(bound);
    }

    fn start_css_images_discovered_by_layout(
        &mut self,
        references: Vec<moli_layout::LayoutCssImageReference<DomHandle>>,
    ) {
        if references.is_empty() {
            return;
        }
        let image_fetch_enabled = self
            .document_runtime
            .current_document_resource_loader()
            .is_some_and(|loader| {
                loader
                    .request_client()
                    .optional_resource_fetch_enabled(crate::types::SubresourceResourceType::Image)
            });
        if !image_fetch_enabled {
            return;
        }
        let mut seen = std::collections::HashSet::new();
        let bound = {
            let mut host = self._context_host.borrow_mut();
            references
                .into_iter()
                .filter_map(|reference| {
                    let document = host.layout_document_for_source(reference.source)?;
                    if !seen.insert((document, reference.resolved_url.clone())) {
                        return None;
                    }
                    if host.stylesheet_css_image_is_current(document, &reference.resolved_url) {
                        return None;
                    }
                    let request_url = url::Url::parse(&reference.resolved_url).ok()?;
                    let binding = host
                        .accept_current_stylesheet_subresource_load_delay_for_document(document)?;
                    Some((
                        binding,
                        crate::css_resource_urls::StylesheetLoadBlockingResource::image(
                            request_url,
                        ),
                    ))
                })
                .collect::<Vec<_>>()
        };
        self.start_stylesheet_subresource_fetches(bound);
    }

    pub(crate) fn observable_geometry_batch_for_current_document(
        &mut self,
        reason: moli_layout::LayoutFlushReason,
        batch: &moli_layout::LayoutQueryBatch<DomHandle>,
    ) -> Result<moli_layout::LayoutAnswers<DomHandle>, moli_layout::LayoutError> {
        let document = self._context_host.borrow().document_handle();
        self.observable_geometry_batch_for_document(document, reason, batch)
    }

    pub(crate) fn observable_geometry_batch_for_document(
        &mut self,
        document: DomHandle,
        reason: moli_layout::LayoutFlushReason,
        batch: &moli_layout::LayoutQueryBatch<DomHandle>,
    ) -> Result<moli_layout::LayoutAnswers<DomHandle>, moli_layout::LayoutError> {
        if self
            ._context_host
            .borrow()
            .layout_policy()
            .uses_real_layout()
        {
            self.reconcile_document_web_fonts_for_layout();
        }
        let host = self._context_host.borrow();
        crate::native_bridge::element::observable_geometry_batch(&host, document, reason, batch)
    }

    pub(crate) fn observable_deep_hit_test_for_current_document(
        &mut self,
        point: moli_layout::LayoutPoint,
        ignore_pointer_events_none: bool,
    ) -> Result<Option<DomHandle>, moli_layout::LayoutError> {
        if self
            ._context_host
            .borrow()
            .layout_policy()
            .uses_real_layout()
        {
            self.reconcile_document_web_fonts_for_layout();
        }
        let host = self._context_host.borrow();
        let document = host.document_handle();
        crate::native_bridge::element::observable_deep_hit_test(
            &host,
            document,
            point,
            ignore_pointer_events_none,
        )
    }

    fn complete_document_web_font(
        &mut self,
        terminal: crate::css_resource_urls::CompletedStylesheetWebFont,
    ) {
        match self
            ._context_host
            .borrow()
            .complete_document_web_font(terminal)
        {
            web_fonts::DocumentWebFontCompletion::Registered(outcome) => tracing::debug!(
                ?outcome,
                "registered current document web font for the next fresh layout refresh"
            ),
            web_fonts::DocumentWebFontCompletion::Invalid(error) => tracing::warn!(
                %error,
                "discarded invalid current document web font response"
            ),
            web_fonts::DocumentWebFontCompletion::NetworkFailed => {
                tracing::debug!("current document web font request reached a failed terminal")
            }
            web_fonts::DocumentWebFontCompletion::Stale => {
                tracing::debug!("discarded superseded document web font response")
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn document_web_font_counts_for_test(&mut self) -> (usize, usize, usize) {
        self._context_host
            .borrow()
            .document_web_font_counts_for_test()
    }

    #[cfg(test)]
    pub(crate) fn normalized_layout_box_tree_for_test(&self) -> anyhow::Result<Option<String>> {
        let context_host = self._context_host.borrow();
        let Some(root) = context_host.dom_host().document_element_handle() else {
            return Ok(None);
        };
        crate::layout_renderer::build_normalized_native_box_tree_for_test(&context_host, root)
            .map(|tree| Some(tree.to_string()))
            .map_err(anyhow::Error::new)
    }

    #[cfg(test)]
    pub(crate) fn build_layout_pass_for_subtree_for_test(
        &self,
        root: DomHandle,
        request: moli_layout::LayoutPassRequest,
    ) -> Result<moli_layout::LayoutPassResult<DomHandle>, moli_layout::LayoutError> {
        let context_host = self._context_host.borrow();
        let mut services = moli_layout::DocumentLayoutServices::new();
        let mut embedded_document_services = HashMap::new();
        crate::layout_renderer::build_native_layout_pass(
            &context_host,
            root,
            &mut services,
            &mut embedded_document_services,
            request,
        )
    }

    pub(super) fn computed_style_properties_for_inspector_handle(
        &self,
        handle: DomHandle,
    ) -> Option<Vec<(String, String)>> {
        crate::native_bridge::element::computed_style_properties_for_inspector_handle(
            &self._context_host.borrow(),
            handle,
        )
    }

    pub(super) fn marker_pseudo_element_is_generated_for_document_snapshot(
        &self,
        handle: DomHandle,
    ) -> bool {
        crate::native_bridge::element::marker_pseudo_element_is_generated_for_document_snapshot(
            &self._context_host.borrow(),
            handle,
        )
        .unwrap_or(false)
    }

    pub(crate) fn owner_style_sheet_text(&self, owner: DomHandle) -> Option<String> {
        self._context_host.borrow().owner_style_sheet_text(owner)
    }

    pub(crate) fn linked_stylesheet_source_for_owner(
        &self,
        owner: DomHandle,
    ) -> Option<crate::style_engine::StyloStylesheetSource> {
        self._context_host
            .borrow()
            .linked_stylesheet_source_for_owner(owner)
    }

    pub(crate) fn stylesheet_owner_is_csp_blocked(&self, owner: DomHandle) -> bool {
        self._context_host
            .borrow()
            .stylesheet_owner_is_csp_blocked(owner)
    }

    pub(crate) fn drain_parser_defined_autonomous_custom_elements(&mut self) -> Vec<String> {
        self._context_host
            .borrow_mut()
            .drain_parser_defined_autonomous_custom_elements()
    }

    #[cfg(test)]
    pub(crate) fn document_handle_for_test(&self) -> DomHandle {
        self.document_runtime.document_handle()
    }

    #[cfg(test)]
    pub(crate) fn pending_style_invalidation_work_item_count_for_current_document_for_test(
        &self,
    ) -> usize {
        let document = self.document_runtime.document_handle();
        self._context_host
            .borrow()
            .pending_style_invalidation_work_item_count_for_document_for_test(document)
    }

    #[cfg(test)]
    pub(crate) fn element_handle_by_id_for_test(&self, id: &str) -> Option<DomHandle> {
        self._context_host
            .borrow()
            .dom_host()
            .element_handle_by_id(id)
    }

    #[cfg(test)]
    pub(crate) fn inline_style_base_url_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> usize {
        self._context_host
            .borrow()
            .inline_style_base_url_count_for_document_for_test(document)
    }

    #[cfg(test)]
    pub(crate) fn computed_style_cache_entry_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> usize {
        self._context_host
            .borrow()
            .computed_style_cache_entry_count_for_document_for_test(document)
    }

    #[cfg(test)]
    pub(crate) fn active_document_style_world_count_for_test(&self) -> usize {
        self._context_host
            .borrow()
            .active_document_style_world_count_for_test()
    }

    #[cfg(test)]
    pub(crate) fn document_style_world_is_active_for_test(&self, document: DomHandle) -> bool {
        self._context_host
            .borrow()
            .document_style_world_is_active_for_test(document)
    }

    #[cfg(test)]
    pub(crate) fn computed_style_cache_generation_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self._context_host
            .borrow()
            .computed_style_cache_generation_for_document_for_test(document)
    }

    #[cfg(test)]
    pub(crate) fn retained_style_system_rebuild_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self._context_host
            .borrow()
            .retained_style_system_rebuild_count_for_document_for_test(document)
    }

    #[cfg(test)]
    pub(crate) fn retained_style_system_update_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self._context_host
            .borrow()
            .retained_style_system_update_count_for_document_for_test(document)
    }

    #[cfg(test)]
    pub(crate) fn retained_stylist_identity_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self._context_host
            .borrow()
            .retained_stylist_identity_for_document_for_test(document)
    }

    #[cfg(test)]
    pub(crate) fn retained_shadow_scope_flush_count_for_document_for_test(
        &self,
        document: DomHandle,
        root: DomHandle,
    ) -> Option<u64> {
        self._context_host
            .borrow()
            .retained_shadow_scope_flush_count_for_document_for_test(document, root)
    }

    #[cfg(test)]
    pub(crate) fn shared_worker_client_count_for_test(&self) -> usize {
        self._context_host
            .borrow()
            .shared_worker_client_count_for_test()
    }

    pub(super) fn service_worker_client_id(
        &self,
    ) -> crate::service_worker_runtime::ServiceWorkerClientId {
        self._context_host.borrow().service_worker_client_id()
    }

    #[cfg(test)]
    pub(crate) fn custom_element_registry_association_count_for_test(&self) -> usize {
        self._context_host
            .borrow()
            .custom_element_registry_association_count_for_test()
    }

    #[cfg(test)]
    pub(crate) fn storage_bucket_keys_for_test(&self, storage_key: &str) -> Vec<String> {
        self.storage_bucket_store.lock().keys(storage_key)
    }

    #[cfg(test)]
    pub(crate) fn context_host_weak_for_test(&self) -> std::rc::Weak<RefCell<JsContextHost>> {
        Rc::downgrade(&self._context_host)
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

    fn child_frame_realm_context_ptr(
        &self,
        realm_id: FrameRealmId,
    ) -> Result<*const v8::Global<v8::Context>> {
        self.child_frame_realm_store
            .context_for_owner_realm_id(realm_id)
            .map(|realm| &realm.context as *const _)
            .ok_or_else(|| anyhow!("unknown child frame owner realm `{realm_id:?}`"))
    }

    fn child_frame_realm_context_ptr_for_execution_context_id(
        &self,
        execution_context_id: i64,
    ) -> Result<*const v8::Global<v8::Context>> {
        let realm_id = self
            .child_frame_realm_store
            .owner_realm_id_for_context_id(execution_context_id)
            .ok_or_else(|| anyhow!("unknown child frame realm `{execution_context_id}`"))?;
        self.child_frame_realm_context_ptr(realm_id)
    }

    fn inspector_window_dispatch_scope_for_target(
        &self,
        target: InspectorWindowDispatchTarget,
    ) -> Option<InspectorWindowDispatchScope> {
        let execution_context_id = match target {
            InspectorWindowDispatchTarget::DefaultTop => {
                return Some(InspectorWindowDispatchScope {
                    context_ptr: &self.page_default_context as *const _,
                    child_handle: None,
                });
            }
            InspectorWindowDispatchTarget::ExecutionContext(execution_context_id) => {
                execution_context_id
            }
        };
        if self.runtime_observable_default_execution_context_id() == Some(execution_context_id) {
            return Some(InspectorWindowDispatchScope {
                context_ptr: &self.page_default_context as *const _,
                child_handle: None,
            });
        }
        if let Some(realm) = self.child_frame_realm_store.get(&execution_context_id) {
            return Some(InspectorWindowDispatchScope {
                context_ptr: &realm.context as *const _,
                child_handle: Some(realm.child_handle),
            });
        }
        let isolated_context_id = self
            .page_isolated_world_contexts
            .execution_context_id_for_inspector_context(execution_context_id)?;
        let world = self
            .page_isolated_world_contexts
            .context(isolated_context_id)?;
        Some(InspectorWindowDispatchScope {
            context_ptr: &world.context as *const _,
            child_handle: world.child_handle,
        })
    }

    fn frame_realm_context_ptr(
        &self,
        realm_id: FrameRealmId,
    ) -> Result<*const v8::Global<v8::Context>> {
        if realm_id.0 == 0 {
            return Ok(&self.page_default_context as *const _);
        }
        self.child_frame_realm_context_ptr(realm_id)
    }

    fn child_frame_owner_realm_id_for_execution_context_id(
        &self,
        execution_context_id: i64,
    ) -> Result<FrameRealmId> {
        let context = self
            .child_frame_realm_store
            .get(&execution_context_id)
            .ok_or_else(|| anyhow!("unknown child frame realm `{execution_context_id}`"))?;
        let host = self._context_host.borrow();
        let owner_realm_id = context.owner_realm_id;
        let owner = host
            .frame_owner_current_child_snapshot_for_realm(owner_realm_id)
            .ok_or_else(|| {
                anyhow!(
                    "child frame realm `{execution_context_id}` has no current FrameOwnerStore snapshot"
                )
            })?;
        if owner.owner_handle != context.child_handle
            || owner.frame_id.0.as_str() != context.frame_id
        {
            return Err(anyhow!(
                "child frame realm `{execution_context_id}` maps to stale owner realm {owner_realm_id:?}"
            ));
        }
        Ok(owner_realm_id)
    }

    fn with_child_frame_realm_context_scope<T>(
        &mut self,
        execution_context_id: i64,
        op: impl FnOnce(&mut v8::PinScope<'_, '_>, *mut JsContextHost) -> Result<T>,
    ) -> Result<T> {
        let realm_id =
            self.child_frame_owner_realm_id_for_execution_context_id(execution_context_id)?;
        self.with_frame_realm_scope(realm_id, op)
    }

    fn child_frame_source_script_job_for_execution_context_id(
        &self,
        execution_context_id: i64,
        kind: FrameScriptJobKind,
        source: String,
    ) -> Result<FrameScriptJob> {
        let context = self
            .child_frame_realm_store
            .get(&execution_context_id)
            .ok_or_else(|| anyhow!("unknown child frame realm `{execution_context_id}`"))?;
        self._context_host
            .borrow()
            .frame_owner_child_source_script_job(context.child_handle, kind, source)
            .ok_or_else(|| {
                anyhow!(
                    "child frame realm `{execution_context_id}` has no current FrameScriptJob owner"
                )
            })
    }

    fn exec_child_frame_source_script_job_for_execution_context_id(
        &mut self,
        execution_context_id: i64,
        kind: FrameScriptJobKind,
        source: &str,
    ) -> Result<()> {
        let job = self.child_frame_source_script_job_for_execution_context_id(
            execution_context_id,
            kind,
            source.to_owned(),
        )?;
        self.exec_frame_script_job(job)
    }

    pub(super) fn default_execution_context_id(&self) -> Option<i64> {
        self.page_inspector.default_execution_context_id()
    }

    pub(super) fn default_or_initial_execution_context_id(&self) -> Option<i64> {
        self.page_inspector
            .default_execution_context_id()
            .or_else(|| self.page_inspector.initial_default_execution_context_id())
    }

    fn runtime_observable_default_execution_context_id(&self) -> Option<i64> {
        // Creation-time console output can be observed before Runtime.enable has
        // materialized the session-visible default context id. The renderer
        // still owns the initial V8 context identity, so source items can be
        // emitted immediately instead of being parked as contextless messages.
        self.page_inspector
            .default_execution_context_id()
            .or_else(|| self.page_inspector.initial_default_execution_context_id())
    }

    fn runtime_observable_default_execution_context_realm_id(&self) -> Option<String> {
        self.page_inspector
            .default_execution_context_realm_id()
            .or_else(|| {
                self.page_inspector
                    .initial_default_execution_context_realm_id()
            })
    }

    pub(super) fn root_frame_id(&self) -> Option<&str> {
        self.root_frame_id.as_deref()
    }

    fn default_runtime_realm_info(&self) -> Option<RendererRuntimeRealmInfo> {
        let context_id = self.runtime_observable_default_execution_context_id()?;
        let document_url = self.document_runtime.document_url();
        Some(RendererRuntimeRealmInfo {
            context_id,
            realm_id: self.runtime_observable_default_execution_context_realm_id(),
            frame_id: self.root_frame_id.clone(),
            origin: moli_url::origin_ascii_serialization(document_url),
            name: document_url.as_str().to_owned(),
            is_default: true,
            context_type: "default".to_owned(),
            grant_universal_access: None,
        })
    }

    pub(super) fn runtime_realm_inventory(&mut self) -> Vec<RendererRuntimeRealmInfo> {
        self.prune_stale_child_default_execution_contexts();
        self.known_runtime_realm_inventory()
    }

    pub(crate) fn known_runtime_realm_inventory(&self) -> Vec<RendererRuntimeRealmInfo> {
        let mut realms = Vec::new();
        realms.extend(self.default_runtime_realm_info());

        let mut isolated_context_ids = self
            .page_isolated_world_contexts
            .execution_context_ids()
            .collect::<Vec<_>>();
        isolated_context_ids.sort_unstable();
        realms.extend(
            isolated_context_ids
                .into_iter()
                .filter_map(|context_id| self.isolated_world_runtime_realm_info(context_id)),
        );

        let mut child_context_ids = self
            .child_frame_realm_store
            .execution_context_ids()
            .collect::<Vec<_>>();
        child_context_ids.sort_unstable();
        realms.extend(
            child_context_ids
                .into_iter()
                .filter_map(|context_id| self.child_default_runtime_realm_info(context_id)),
        );

        realms
    }

    fn runtime_inspector_messages_from_v8_messages(
        &self,
        messages: impl IntoIterator<Item = Value>,
    ) -> Vec<RendererRuntimeInspectorMessage> {
        messages
            .into_iter()
            .map(RendererRuntimeInspectorMessage::from_v8_inspector_message)
            .collect()
    }

    pub(super) fn has_isolated_execution_context_id(&self, execution_context_id: i64) -> bool {
        self.page_isolated_world_contexts
            .has_execution_context_id(execution_context_id)
    }

    pub(super) fn has_isolated_world_named(&self, name: &str) -> bool {
        self.page_isolated_world_contexts
            .has_world_for_scope(None, name)
    }

    pub(super) fn has_isolated_world_named_for_frame(&self, frame_id: &str, name: &str) -> bool {
        self.page_isolated_world_contexts
            .has_world_for_scope(Some(frame_id), name)
    }

    pub(super) fn has_isolated_world_for_owner(
        &self,
        devtools_session: Option<&DevToolsSessionKey>,
        frame_id: Option<&str>,
        name: &str,
    ) -> bool {
        self.page_isolated_world_contexts
            .execution_context_id_for_scope(devtools_session, frame_id, name)
            .is_some()
    }

    pub(super) fn inspector_execution_context_id_for_isolated_context(
        &self,
        execution_context_id: i64,
    ) -> Option<i64> {
        self.page_isolated_world_contexts
            .inspector_execution_context_id(execution_context_id)
    }

    pub(super) fn isolated_execution_context_id_for_inspector_context(
        &self,
        execution_context_id: i64,
    ) -> Option<i64> {
        self.page_isolated_world_contexts
            .execution_context_id_for_inspector_context(execution_context_id)
    }

    pub(crate) fn child_default_frame_id_for_execution_context_id(
        &mut self,
        execution_context_id: i64,
    ) -> Option<String> {
        self.prune_stale_child_default_execution_contexts();
        let context = self.child_frame_realm_store.get(&execution_context_id)?;
        self._context_host
            .borrow()
            .frame_owner_frame_id_for_child_handle(context.child_handle)
            .map(|frame_id| frame_id.0)
            .or_else(|| Some(context.frame_id.clone()))
    }

    pub(crate) fn child_default_execution_context_id_for_frame_id(
        &mut self,
        frame_id: &str,
    ) -> Option<i64> {
        self.prune_stale_child_default_execution_contexts();
        self.child_frame_realm_store
            .iter_by_execution_context_id()
            .find_map(|(execution_context_id, context)| {
                (context.frame_id == frame_id).then_some(execution_context_id)
            })
    }

    pub(crate) fn child_browsing_context_module_request_initiator_url(
        &self,
        child_handle: crate::document_runtime::DomHandle,
    ) -> Option<Url> {
        let host = self._context_host.borrow();
        host.child_browsing_context_base_url(child_handle)
            .or_else(|| host.child_browsing_context_current_url(child_handle))
    }

    fn isolated_world_runtime_realm_info(
        &self,
        context_id: i64,
    ) -> Option<RendererRuntimeRealmInfo> {
        let world = self.page_isolated_world_contexts.context(context_id)?;
        let origin = world
            .child_handle
            .and_then(|handle| {
                self._context_host
                    .borrow()
                    .child_browsing_context_current_url(handle)
            })
            .unwrap_or_else(|| self.document_runtime.document_url().clone());
        Some(RendererRuntimeRealmInfo {
            context_id,
            realm_id: world.inspector_execution_context_realm_id.clone(),
            frame_id: world
                .frame_id
                .clone()
                .or_else(|| self.root_frame_id.clone()),
            origin: moli_url::origin_ascii_serialization(&origin),
            name: world.name.clone(),
            is_default: false,
            context_type: "isolated".to_owned(),
            grant_universal_access: Some(world.grant_universal_access),
        })
    }

    fn child_default_runtime_realm_info(
        &self,
        context_id: i64,
    ) -> Option<RendererRuntimeRealmInfo> {
        let context = self.child_frame_realm_store.get(&context_id)?;
        let document_url = self
            ._context_host
            .borrow()
            .child_browsing_context_current_url(context.child_handle)
            .unwrap_or_else(|| self.document_runtime.document_url().clone());
        Some(RendererRuntimeRealmInfo {
            context_id,
            realm_id: context.inspector_execution_context_realm_id.clone(),
            frame_id: Some(context.frame_id.clone()),
            origin: moli_url::origin_ascii_serialization(&document_url),
            name: document_url.as_str().to_owned(),
            is_default: true,
            context_type: "default".to_owned(),
            grant_universal_access: None,
        })
    }

    pub(super) fn live_child_default_runtime_realm_inventory(
        &mut self,
    ) -> Vec<RendererRuntimeRealmInfo> {
        self.prune_stale_child_default_execution_contexts();
        let mut child_context_ids = self
            .child_frame_realm_store
            .execution_context_ids()
            .collect::<Vec<_>>();
        child_context_ids.sort_unstable();
        child_context_ids
            .into_iter()
            .filter_map(|context_id| self.child_default_runtime_realm_info(context_id))
            .collect()
    }

    fn live_child_default_context_entries(&self) -> Vec<LiveChildDefaultContextEntry> {
        let host = self._context_host.borrow();
        host.live_child_browsing_context_owner_snapshots()
            .into_iter()
            .map(|(handle, owner)| LiveChildDefaultContextEntry {
                handle,
                frame_id: owner.frame_id.0,
                owner_realm_id: owner.realm_id,
            })
            .collect()
    }

    fn create_new_child_default_world(
        &mut self,
        frame_id: &str,
        child_handle: DomHandle,
    ) -> Result<ChildFrameRealmRecord> {
        let current_owner = self
            ._context_host
            .borrow()
            .current_child_document_task_owner(child_handle)
            .ok_or_else(|| anyhow!("child frame `{frame_id}` has no current LocalWindow owner"))?;
        let prebootstrapped = self
            .prebootstrapped_child_default_contexts
            .borrow_mut()
            .remove(&child_handle);
        let prebootstrapped = match prebootstrapped {
            Some(context) if context.local_window_id == current_owner.local_window_id => {
                Some(context)
            }
            Some(context) => {
                self._context_host
                    .borrow_mut()
                    .retire_window_execution_contexts_for_context_token(
                        context.runtime_observable_context_token,
                    );
                let context_ptr = &context.context as *const v8::Global<v8::Context>;
                self.renderer_document_isolate
                    .with_entered_renderer_document_isolate(|isolate| {
                        let scope = pin!(v8::HandleScope::new(isolate));
                        let scope = &mut scope.init();
                        let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                        context.detach_global();
                        Ok(())
                    })?;
                None
            }
            None => None,
        };
        let context_host = self._context_host.clone();
        let (context, bridge_ref, runtime_observable_context_token) =
            if let Some(prebootstrapped) = prebootstrapped {
                (
                    prebootstrapped.context,
                    prebootstrapped.bridge_ref,
                    prebootstrapped.runtime_observable_context_token,
                )
            } else {
                let context_bootstrap = self
                    .renderer_document_isolate
                    .with_entered_renderer_document_isolate_and_bootstrap(
                        |isolate, isolate_bootstrap| {
                            ScriptVmContextBootstrap::new_child_default(
                                isolate,
                                isolate_bootstrap,
                                context_host,
                                self.resource_owner_id,
                                &self.promise_reject_dispatch,
                                self.indexed_db_manager.clone(),
                                Some(self.storage_bucket_store.clone()),
                                child_handle,
                                current_owner,
                            )
                        },
                    )?;
                let runtime_observable_context_token =
                    context_bootstrap.runtime_observable_context_token;
                let (context, bridge_ref) = context_bootstrap.into_context_and_bridge_ref();
                (context, bridge_ref, runtime_observable_context_token)
            };
        let document_url = self
            ._context_host
            .borrow()
            .child_browsing_context_current_url(child_handle)
            .unwrap_or_else(|| self.document_runtime.document_url().clone());
        let renderer_document_isolate = self.renderer_document_isolate.clone();
        let inspector_document_isolate = renderer_document_isolate.clone();
        let page_inspector = &mut self.page_inspector;
        let (inspector_context, inspector_context_registration_id) = renderer_document_isolate
            .with_entered_renderer_document_isolate_and_inspector_mut(|isolate, inspector| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let local_context = v8::Local::new(scope, &context);
                let registered_context = v8::Global::new(scope.as_ref(), local_context);
                page_inspector.attach_child_default_context(
                    inspector_document_isolate,
                    inspector,
                    local_context,
                    registered_context,
                    &document_url,
                    frame_id,
                )
            })?;
        let owner_realm_id = self
            ._context_host
            .borrow_mut()
            .bind_child_default_execution_context_id(
                child_handle,
                current_owner,
                inspector_context.id,
            );
        let Some(owner_realm_id) = owner_realm_id else {
            assert!(
                self.page_inspector
                    .destroy_context_registration(inspector_context_registration_id),
                "failed child realm materialization must release its Inspector registration"
            );
            return Err(anyhow!(
                "child frame `{frame_id}` has no current FrameOwnerStore record for FrameRealm materialization"
            ));
        };
        Ok(ChildFrameRealmRecord {
            frame_id: frame_id.to_owned(),
            child_handle,
            local_window_id: current_owner.local_window_id,
            owner_realm_id,
            context,
            _bridge_ref: bridge_ref,
            runtime_observable_context_token,
            inspector_execution_context_id: inspector_context.id,
            inspector_execution_context_realm_id: inspector_context.unique_id,
            inspector_context_registration_id,
        })
    }

    fn prune_stale_child_default_execution_contexts(&mut self) {
        let live = self.live_child_default_context_entries();
        self.prune_stale_child_default_execution_contexts_for_live_entries(&live);
    }

    fn prune_stale_child_default_execution_contexts_for_live_entries(
        &mut self,
        live: &[LiveChildDefaultContextEntry],
    ) {
        let stale_prebootstrapped_handles = {
            let host = self._context_host.borrow();
            self.prebootstrapped_child_default_contexts
                .borrow()
                .iter()
                .filter_map(|(handle, context)| {
                    (host
                        .current_child_document_task_owner(*handle)
                        .map(|owner| owner.local_window_id)
                        != Some(context.local_window_id))
                    .then_some(*handle)
                })
                .collect::<Vec<_>>()
        };
        let stale_prebootstrapped_contexts = {
            let mut contexts = self.prebootstrapped_child_default_contexts.borrow_mut();
            stale_prebootstrapped_handles
                .into_iter()
                .filter_map(|handle| contexts.remove(&handle))
                .collect::<Vec<_>>()
        };
        if !stale_prebootstrapped_contexts.is_empty() {
            {
                let mut host = self._context_host.borrow_mut();
                for context in &stale_prebootstrapped_contexts {
                    host.retire_window_execution_contexts_for_context_token(
                        context.runtime_observable_context_token,
                    );
                }
            }
            let _ = self
                .renderer_document_isolate
                .with_entered_renderer_document_isolate(|isolate| {
                    let scope = pin!(v8::HandleScope::new(isolate));
                    let scope = &mut scope.init();
                    for context in &stale_prebootstrapped_contexts {
                        v8::Local::new(scope, &context.context).detach_global();
                    }
                    Ok(())
                });
        }
        let stale_context_ids = self
            .child_frame_realm_store
            .iter_by_execution_context_id()
            .filter_map(|(context_id, context)| {
                let live_entry = live
                    .iter()
                    .find(|entry| entry.handle == context.child_handle);
                let is_stale = live_entry
                    .map(|entry| entry.owner_realm_id != Some(context.owner_realm_id))
                    .unwrap_or(true)
                    || live_entry
                        .map(|entry| entry.frame_id != context.frame_id)
                        .unwrap_or(true)
                    || self
                        .child_frame_realm_store
                        .context_id_for_owner_realm_id(context.owner_realm_id)
                        != Some(context_id);
                is_stale.then_some(context_id)
            })
            .collect::<Vec<_>>();
        for context_id in stale_context_ids {
            self.destroy_child_default_context(context_id);
        }
    }

    fn destroy_child_default_context(&mut self, execution_context_id: i64) {
        let Some(context) = self.child_frame_realm_store.remove(&execution_context_id) else {
            return;
        };
        let retired_timer_count = self
            .document_runtime
            .cancel_timers_for_context_token(context.runtime_observable_context_token);
        let runtime_binding_retirement = {
            let mut host = self._context_host.borrow_mut();
            host.clear_child_default_execution_context_if_matches(
                context.child_handle,
                context.owner_realm_id,
                execution_context_id,
            );
            let runtime_binding_retirement =
                host.retire_runtime_binding_context_token(context.runtime_observable_context_token);
            let retired_image_decode_count = host.retire_image_decode_requests_for_context_token(
                context.runtime_observable_context_token,
            );
            let retired_webcrypto_count =
                host.retire_webcrypto_context_token(context.runtime_observable_context_token);
            host.retire_opfs_context_token(context.runtime_observable_context_token);
            let retired_worker_count =
                host.retire_workers_for_context_token(context.runtime_observable_context_token);
            let retired_shared_worker_count = host
                .disconnect_shared_worker_clients_for_context_token(
                    context.runtime_observable_context_token,
                );
            let retired_xhr_count =
                host.retire_window_xhrs_for_context_token(context.runtime_observable_context_token);
            let retired_fetch_count = host
                .retire_window_fetches_for_context_token(context.runtime_observable_context_token);
            host.retire_window_event_sources_for_context_token(
                context.runtime_observable_context_token,
            );
            let retired_message_port_count = host
                .retire_message_ports_for_context_token(context.runtime_observable_context_token);
            let retired_window_message_count = host
                .retire_window_messages_for_context_token(context.runtime_observable_context_token);
            let retired_window_execution_context_count = host
                .retire_window_execution_contexts_for_context_token(
                    context.runtime_observable_context_token,
                );
            (
                runtime_binding_retirement,
                retired_image_decode_count,
                retired_message_port_count,
                retired_window_message_count,
                retired_window_execution_context_count,
                retired_webcrypto_count,
                retired_worker_count,
                retired_shared_worker_count,
                retired_xhr_count,
                retired_fetch_count,
            )
        };
        tracing::debug!(
            execution_context_id,
            context_token = ?context.runtime_observable_context_token,
            retired_runtime_binding_context_count = runtime_binding_retirement.0
                .retired_execution_context_count(),
            retired_image_decode_count = runtime_binding_retirement.1,
            retired_message_port_count = runtime_binding_retirement.2,
            retired_window_message_count = runtime_binding_retirement.3,
            retired_window_execution_context_count = runtime_binding_retirement.4,
            retired_webcrypto_count = runtime_binding_retirement.5,
            retired_worker_count = runtime_binding_retirement.6,
            retired_shared_worker_count = runtime_binding_retirement.7,
            retired_xhr_count = runtime_binding_retirement.8,
            aborted_fetch_count = runtime_binding_retirement.9.0,
            detached_keepalive_fetch_count = runtime_binding_retirement.9.1,
            retired_timer_count,
            "retired child Runtime binding context"
        );
        let context_ptr: *const v8::Global<v8::Context> = &context.context as *const _;
        self.clear_context_wrapper_cache_for_context_ptr(context_ptr, false);
        assert!(
            self.page_inspector
                .destroy_context_registration(context.inspector_context_registration_id),
            "child default context must retain its document-owned Inspector registration"
        );
        let context_host = self._context_host.clone();
        let detach_result = self
            .renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let local_context = unsafe { v8::Local::new(scope, &*context_ptr) };
                local_context.detach_global();
                let host_ptr = (*context_host).as_ptr();
                let host = unsafe { &mut *host_ptr };
                if host.child_browsing_context_is_live(context.child_handle)
                    && !host.preserve_child_window_proxy_between_realms(scope, context.child_handle)
                {
                    anyhow::bail!("failed to park the live child WindowProxy between realms");
                }
                Ok(())
            });
        if let Err(error) = detach_result {
            tracing::warn!(
                %error,
                execution_context_id,
                child_handle = context.child_handle.index(),
                owner_realm_id = ?context.owner_realm_id,
                "failed to detach retired child WindowProxy global"
            );
        } else {
            tracing::debug!(
                execution_context_id,
                child_handle = context.child_handle.index(),
                owner_realm_id = ?context.owner_realm_id,
                "detached retired child WindowProxy global for identity reuse"
            );
        }
    }

    fn destroy_isolated_world_context(&mut self, execution_context_id: i64) {
        let Some(context) = self
            .page_isolated_world_contexts
            .remove_context(execution_context_id)
        else {
            return;
        };
        let retired_timer_count = self
            .document_runtime
            .cancel_timers_for_context_token(context.runtime_observable_context_token);
        let (
            runtime_binding_retirement,
            retired_image_decode_count,
            retired_message_port_count,
            retired_webcrypto_count,
            retired_worker_count,
            retired_shared_worker_count,
            retired_xhr_count,
            retired_fetch_count,
        ) = {
            let mut host = self._context_host.borrow_mut();
            let runtime_binding_retirement =
                host.retire_runtime_binding_context_token(context.runtime_observable_context_token);
            let retired_image_decode_count = host.retire_image_decode_requests_for_context_token(
                context.runtime_observable_context_token,
            );
            let retired_webcrypto_count =
                host.retire_webcrypto_context_token(context.runtime_observable_context_token);
            host.retire_opfs_context_token(context.runtime_observable_context_token);
            let retired_worker_count =
                host.retire_workers_for_context_token(context.runtime_observable_context_token);
            let retired_shared_worker_count = host
                .disconnect_shared_worker_clients_for_context_token(
                    context.runtime_observable_context_token,
                );
            let retired_xhr_count =
                host.retire_window_xhrs_for_context_token(context.runtime_observable_context_token);
            let retired_fetch_count = host
                .retire_window_fetches_for_context_token(context.runtime_observable_context_token);
            host.retire_window_event_sources_for_context_token(
                context.runtime_observable_context_token,
            );
            let retired_message_port_count = host
                .retire_message_ports_for_context_token(context.runtime_observable_context_token);
            (
                runtime_binding_retirement,
                retired_image_decode_count,
                retired_message_port_count,
                retired_webcrypto_count,
                retired_worker_count,
                retired_shared_worker_count,
                retired_xhr_count,
                retired_fetch_count,
            )
        };
        assert!(
            self.page_inspector
                .destroy_context_registration(context.inspector_context_registration_id),
            "isolated context must retain its document-owned Inspector registration"
        );
        // V8 Inspector consumes the still-identifiable realm while processing
        // `context_destroyed` (including Runtime lifecycle projection for
        // named child worlds). Only after that notification may the strict
        // realm locator become stale. Isolated worlds have no owner-indexed
        // default-world binding, so retire only this token's registry entry.
        let retired_window_execution_context_realm_count = self
            ._context_host
            .borrow_mut()
            .retire_isolated_window_execution_context(context.runtime_observable_context_token);
        tracing::debug!(
            execution_context_id,
            context_token = ?context.runtime_observable_context_token,
            retired_runtime_binding_context_count = runtime_binding_retirement
                .retired_execution_context_count(),
            retired_image_decode_count,
            retired_message_port_count,
            retired_webcrypto_count,
            retired_worker_count,
            retired_shared_worker_count,
            retired_xhr_count,
            aborted_fetch_count = retired_fetch_count.0,
            detached_keepalive_fetch_count = retired_fetch_count.1,
            retired_window_execution_context_realm_count,
            retired_timer_count,
            "retired isolated-world Runtime binding context"
        );
    }

    pub(super) fn retire_isolated_worlds_for_document_owner(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> usize {
        let stale_context_ids = self
            .page_isolated_world_contexts
            .contexts_with_ids()
            .filter_map(|(context_id, world)| (world.document_owner == owner).then_some(context_id))
            .collect::<Vec<_>>();
        let retired_count = stale_context_ids.len();
        for context_id in stale_context_ids {
            self.destroy_isolated_world_context(context_id);
        }
        retired_count
    }

    fn retire_isolated_worlds_for_devtools_session(
        &mut self,
        devtools_session: &DevToolsSessionKey,
    ) -> usize {
        let context_ids = self
            .page_isolated_world_contexts
            .execution_context_ids_for_devtools_session(devtools_session);
        let retired_count = context_ids.len();
        for context_id in context_ids {
            self.destroy_isolated_world_context(context_id);
        }
        retired_count
    }

    pub(super) fn rebind_isolated_worlds_for_document_owner_transition(
        &mut self,
        retired_owner: FrameDocumentTaskOwner,
        current_owner: FrameDocumentTaskOwner,
    ) -> usize {
        let targets = self
            .page_isolated_world_contexts
            .contexts_with_ids()
            .filter_map(|(execution_context_id, world)| {
                (world.document_owner == retired_owner).then_some((
                    execution_context_id,
                    world.child_handle,
                    world.runtime_observable_context_token,
                ))
            })
            .collect::<Vec<_>>();
        let mut rebound_count = 0;
        for (execution_context_id, child_handle, realm_token) in targets {
            let rebound = if let Some(child_handle) = child_handle {
                let Some(context_ptr) = self
                    .page_isolated_world_contexts
                    .context(execution_context_id)
                    .map(|world| &world.context as *const v8::Global<v8::Context>)
                else {
                    continue;
                };
                let context_host = self._context_host.clone();
                let result = self
                    .renderer_document_isolate
                    .with_entered_renderer_document_isolate(|isolate| {
                        let scope = pin!(v8::HandleScope::new(isolate));
                        let scope = &mut scope.init();
                        let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                        let child_scope = &mut v8::ContextScope::new(scope, context);
                        let global = context.global(child_scope);
                        context_host
                            .borrow_mut()
                            .rebind_child_window_realm_document_state(
                                child_scope,
                                global,
                                child_handle,
                                retired_owner,
                                current_owner,
                                realm_token,
                            )
                    });
                if let Err(error) = result {
                    tracing::warn!(
                        execution_context_id,
                        ?child_handle,
                        ?retired_owner,
                        ?current_owner,
                        %error,
                        "failed closed while rebinding isolated-world document state"
                    );
                    false
                } else {
                    true
                }
            } else {
                true
            };
            if !rebound {
                continue;
            }
            if let Some(world) = self
                .page_isolated_world_contexts
                .context_mut(execution_context_id)
                && world.document_owner == retired_owner
            {
                world.document_owner = current_owner;
                rebound_count += 1;
            }
        }
        rebound_count
    }

    fn create_new_isolated_world(
        &mut self,
        devtools_session: Option<DevToolsSessionKey>,
        name: &str,
        grant_universal_access: bool,
        frame_id: Option<String>,
        child_handle: Option<DomHandle>,
    ) -> Result<i64> {
        let document_owner = match child_handle {
            Some(child_handle) => self
                ._context_host
                .borrow()
                .current_child_document_task_owner(child_handle)
                .ok_or_else(|| {
                    anyhow!(
                        "cannot create isolated world for child without a current document owner"
                    )
                })?,
            None => self
                .current_main_document_task_owner()
                .ok_or_else(|| anyhow!("cannot create isolated world without a main document"))?,
        };
        let context_host = self._context_host.clone();
        let context_bootstrap = self
            .renderer_document_isolate
            .with_entered_renderer_document_isolate_and_bootstrap(
                |isolate, isolate_bootstrap| {
                    ScriptVmContextBootstrap::new_isolated(
                        isolate,
                        isolate_bootstrap,
                        context_host,
                        self.resource_owner_id,
                        &self.promise_reject_dispatch,
                        self.indexed_db_manager.clone(),
                        Some(self.storage_bucket_store.clone()),
                        child_handle,
                        document_owner,
                        if grant_universal_access {
                            crate::native_bridge::WindowExecutionContextAccessPolicy::Universal
                        } else {
                            crate::native_bridge::WindowExecutionContextAccessPolicy::EnforceWebOrigin
                        },
                    )
                },
            )?;
        let runtime_observable_context_token = context_bootstrap.runtime_observable_context_token;
        let (context, bridge_ref) = context_bootstrap.into_context_and_bridge_ref();
        let renderer_document_isolate = self.renderer_document_isolate.clone();
        let inspector_document_isolate = renderer_document_isolate.clone();
        let page_inspector = &mut self.page_inspector;
        let inspector_frame_id = frame_id.as_deref().or(self.root_frame_id.as_deref());
        let inspector_context = renderer_document_isolate
            .with_entered_renderer_document_isolate_and_inspector_mut(|isolate, inspector| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let local_context = v8::Local::new(scope, &context);
                let registered_context = v8::Global::new(scope.as_ref(), local_context);
                page_inspector.attach_isolated_context(
                    inspector_document_isolate,
                    inspector,
                    local_context,
                    registered_context,
                    None,
                    name,
                    grant_universal_access,
                    inspector_frame_id,
                )
            })?;
        let (inspector_context, inspector_context_registration_id) = inspector_context
            .ok_or_else(|| anyhow!("V8 inspector did not report isolated execution context id"))?;
        let execution_context_id = inspector_context.id;
        assert!(
            !self
                .page_isolated_world_contexts
                .has_execution_context_id(execution_context_id),
            "new isolated world reused a live execution context id"
        );
        let replaced_context = self.page_isolated_world_contexts.insert_context(
            execution_context_id,
            PageIsolatedWorldContext {
                name: name.to_owned(),
                devtools_session,
                grant_universal_access,
                frame_id,
                child_handle,
                document_owner,
                context,
                _bridge_ref: bridge_ref,
                runtime_observable_context_token,
                inspector_execution_context_id: Some(inspector_context.id),
                inspector_execution_context_realm_id: inspector_context.unique_id,
                inspector_context_registration_id,
            },
        );
        debug_assert!(replaced_context.is_none());
        let isolated_dispatch_scope = child_handle
            .map(crate::native_bridge::OwnerDispatchScope::Child)
            .unwrap_or(crate::native_bridge::OwnerDispatchScope::Top);
        let isolated_execution_context_owner =
            crate::native_bridge::WindowExecutionContextOwner::Frame(
                document_owner.local_window_id,
            );
        if !self
            ._context_host
            .borrow_mut()
            .register_window_execution_context_realm(
                isolated_execution_context_owner,
                isolated_dispatch_scope,
                runtime_observable_context_token,
                if grant_universal_access {
                    crate::native_bridge::WindowExecutionContextAccessPolicy::Universal
                } else {
                    crate::native_bridge::WindowExecutionContextAccessPolicy::EnforceWebOrigin
                },
            )
        {
            let context = self
                .page_isolated_world_contexts
                .remove_context(execution_context_id);
            if let Some(context) = context {
                self.page_inspector
                    .destroy_context_registration(context.inspector_context_registration_id);
            }
            return Err(anyhow!(
                "failed to register isolated Window execution-context realm"
            ));
        }
        if let Some(child_handle) = child_handle {
            let context_ptr = self
                .page_isolated_world_contexts
                .context(execution_context_id)
                .map(|world| &world.context as *const v8::Global<v8::Context>)
                .ok_or_else(|| {
                    anyhow!("unknown isolated execution context `{execution_context_id}`")
                })?;
            let context_host = self._context_host.clone();
            self.renderer_document_isolate
                .with_entered_renderer_document_isolate(|isolate| {
                    let scope = pin!(v8::HandleScope::new(isolate));
                    let scope = &mut scope.init();
                    let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                    let child_scope = &mut v8::ContextScope::new(scope, context);
                    context_host
                        .borrow_mut()
                        .bind_child_window_indexed_db_factory_after_context_registration(
                            child_scope,
                            child_handle,
                        );
                    Ok(())
                })?;
        }
        Ok(execution_context_id)
    }

    pub(super) fn ensure_isolated_worlds_attached_to_inspector(&mut self) -> Result<()> {
        let pending_ids = self
            .page_isolated_world_contexts
            .pending_inspector_attachment_ids();
        for execution_context_id in pending_ids {
            let context_ptr: *const v8::Global<v8::Context> = self
                .page_isolated_world_contexts
                .context(execution_context_id)
                .map(|world| &world.context as *const _)
                .ok_or_else(|| {
                    anyhow!("unknown isolated execution context `{execution_context_id}`")
                })?;
            let (name, grant_universal_access, frame_id, replaced_registration_id) = self
                .page_isolated_world_contexts
                .context(execution_context_id)
                .map(|world| {
                    (
                        world.name.clone(),
                        world.grant_universal_access,
                        world.frame_id.clone(),
                        world.inspector_context_registration_id,
                    )
                })
                .ok_or_else(|| {
                    anyhow!("unknown isolated execution context `{execution_context_id}`")
                })?;
            let inspector_frame_id = frame_id.as_deref().or(self.root_frame_id.as_deref());
            let renderer_document_isolate = self.renderer_document_isolate.clone();
            let inspector_document_isolate = renderer_document_isolate.clone();
            let page_inspector = &mut self.page_inspector;
            let inspector_context = renderer_document_isolate
                .with_entered_renderer_document_isolate_and_inspector_mut(
                    |isolate, inspector| {
                        let scope = pin!(v8::HandleScope::new(isolate));
                        let scope = &mut scope.init();
                        let local_context = unsafe { v8::Local::new(scope, &*context_ptr) };
                        let registered_context = v8::Global::new(scope.as_ref(), local_context);
                        page_inspector.attach_isolated_context(
                            inspector_document_isolate,
                            inspector,
                            local_context,
                            registered_context,
                            Some(replaced_registration_id),
                            &name,
                            grant_universal_access,
                            inspector_frame_id,
                        )
                    },
                )?;
            if let Some((inspector_context, registration_id)) = inspector_context {
                if let Some(world) = self
                    .page_isolated_world_contexts
                    .context_mut(execution_context_id)
                {
                    world.inspector_context_registration_id = registration_id;
                }
                self.page_isolated_world_contexts
                    .set_inspector_execution_context_id(
                        execution_context_id,
                        inspector_context.id,
                        inspector_context.unique_id,
                    );
            }
        }
        Ok(())
    }

    pub(super) fn create_isolated_world(
        &mut self,
        name: &str,
        grant_universal_access: bool,
    ) -> Result<i64> {
        self.ensure_isolated_world_for_owner(None, name, grant_universal_access)
    }

    pub(super) fn create_isolated_world_for_frame(
        &mut self,
        frame_id: &str,
        name: &str,
        grant_universal_access: bool,
    ) -> Result<i64> {
        self.ensure_isolated_world_for_frame_owner(None, frame_id, name, grant_universal_access)
    }

    pub(super) fn ensure_isolated_world_for_owner(
        &mut self,
        devtools_session: Option<&DevToolsSessionKey>,
        name: &str,
        grant_universal_access: bool,
    ) -> Result<i64> {
        if let Some(execution_context_id) = self
            .page_isolated_world_contexts
            .execution_context_id_for_scope(devtools_session, None, name)
        {
            let owner_is_current = self
                .page_isolated_world_contexts
                .context(execution_context_id)
                .is_some_and(|world| {
                    self._context_host
                        .borrow()
                        .document_task_owner_is_current(world.document_owner)
                });
            if !owner_is_current {
                return Err(anyhow!(
                    "isolated world `{name}` belongs to a retired main document"
                ));
            }
            return Ok(execution_context_id);
        }
        self.create_new_isolated_world(
            devtools_session.cloned(),
            name,
            grant_universal_access,
            None,
            None,
        )
    }

    pub(super) fn ensure_isolated_world_for_frame_owner(
        &mut self,
        devtools_session: Option<&DevToolsSessionKey>,
        frame_id: &str,
        name: &str,
        grant_universal_access: bool,
    ) -> Result<i64> {
        if let Some(execution_context_id) = self
            .page_isolated_world_contexts
            .execution_context_id_for_scope(devtools_session, Some(frame_id), name)
        {
            let owner_is_current = self
                .page_isolated_world_contexts
                .context(execution_context_id)
                .is_some_and(|world| {
                    self._context_host
                        .borrow()
                        .document_task_owner_is_current(world.document_owner)
                });
            if owner_is_current {
                return Ok(execution_context_id);
            }
            self.destroy_isolated_world_context(execution_context_id);
        }
        let child_handle = self
            ._context_host
            .borrow()
            .child_browsing_context_handle_by_frame_id(frame_id)
            .ok_or_else(|| anyhow!("no live child browsing context for frame `{frame_id}`"))?;
        self.create_new_isolated_world(
            devtools_session.cloned(),
            name,
            grant_universal_access,
            Some(frame_id.to_owned()),
            Some(child_handle),
        )
    }

    pub(super) fn install_runtime_binding(
        &mut self,
        name: &str,
        execution_context_name: Option<&str>,
        execution_context_id: Option<i64>,
    ) -> Result<()> {
        if let Some(execution_context_id) = execution_context_id {
            return self.install_runtime_binding_in_execution_context(execution_context_id, name);
        }
        let Some(execution_context_name) = execution_context_name else {
            return self.install_runtime_binding_in_default_context(name);
        };
        let matching_context_ids = self
            .page_isolated_world_contexts
            .execution_context_ids_for_name(execution_context_name);
        if matching_context_ids.is_empty() {
            return Ok(());
        }

        for execution_context_id in matching_context_ids {
            self.install_runtime_binding_in_isolated_context(execution_context_id, name)?;
        }
        Ok(())
    }

    pub(super) fn remove_runtime_binding(&mut self, name: &str) -> Result<()> {
        self.remove_runtime_binding_from_default_context(name)?;
        let isolated_context_ids = self
            .page_isolated_world_contexts
            .execution_context_ids()
            .collect::<Vec<_>>();
        for execution_context_id in isolated_context_ids {
            self.remove_runtime_binding_from_isolated_context(execution_context_id, name)?;
        }
        self.remove_runtime_binding_from_child_default_contexts(name)?;
        Ok(())
    }

    pub(super) fn remove_default_runtime_binding(&mut self, name: &str) -> Result<()> {
        self.remove_runtime_binding_from_default_context(name)
    }

    pub(super) fn run_document_start_script_now(
        &mut self,
        script: &DocumentStartScript,
    ) -> Result<Option<(i64, bool)>> {
        match script.world_name.as_deref() {
            Some(world_name) => {
                let created = self
                    .page_isolated_world_contexts
                    .execution_context_id_for_scope(
                        script.devtools_session.as_ref(),
                        None,
                        world_name,
                    )
                    .is_none();
                let execution_context_id = self.ensure_isolated_world_for_owner(
                    script.devtools_session.as_ref(),
                    world_name,
                    false,
                )?;
                self.exec_in_execution_context(execution_context_id, &script.source)?;
                Ok(Some((execution_context_id, created)))
            }
            None => {
                self.exec_runtime_turn(&script.source, None)?;
                Ok(None)
            }
        }
    }

    pub(super) fn run_document_start_script_in_execution_context(
        &mut self,
        execution_context_id: i64,
        script: &DocumentStartScript,
    ) -> Result<()> {
        self.exec_in_execution_context(execution_context_id, &script.source)
    }

    pub(crate) fn scroll_live_node_handle_into_view_if_needed(
        &mut self,
        handle: DomHandle,
        rect: Option<moli_page_types::DomScrollIntoViewRect>,
    ) -> Result<RendererScrollIntoViewResult> {
        if !self._context_host.borrow().dom_host().is_connected(handle) {
            return Ok(RendererScrollIntoViewResult::NodeDetached);
        }
        self.with_default_context_scope(|scope, runtime_ptr| {
            Ok(
                match crate::native_bridge::element::scroll_node_into_view_if_needed(
                    scope,
                    runtime_ptr,
                    handle,
                    rect,
                )? {
                    Some(_) => RendererScrollIntoViewResult::ScrolledOrAlreadyVisible,
                    None => RendererScrollIntoViewResult::NodeDoesNotHaveLayoutObject,
                },
            )
        })
    }

    pub(crate) fn client_rect_for_live_node_handle(
        &mut self,
        handle: DomHandle,
    ) -> Result<Option<ClientRect>, moli_layout::LayoutError> {
        let Some(document) = self
            ._context_host
            .borrow()
            .dom_host()
            .owner_document_handle(handle)
        else {
            return Ok(None);
        };
        let answers = self.observable_geometry_batch_for_document(
            document,
            moli_layout::LayoutFlushReason::CdpGeometry,
            &moli_layout::LayoutQueryBatch::new(vec![moli_layout::LayoutQuery::ClientRects {
                source: handle,
            }]),
        )?;
        let Some(moli_layout::LayoutQueryAnswer::ClientRects(mut quads)) =
            answers.answers.into_iter().next()
        else {
            return Err(moli_layout::LayoutError::source_contract(
                "renderer client rect",
                "provider returned a mismatched client-rects answer",
            ));
        };
        if quads.is_empty() {
            return Ok(None);
        }
        self.compose_layout_quads_to_top(document, &mut quads)?;
        let Some(rect) = quads
            .into_iter()
            .map(moli_layout::LayoutQuad::bounding_rect)
            .reduce(moli_layout::LayoutRect::union)
        else {
            return Ok(None);
        };
        Ok(Some(ClientRect {
            left: f64::from(rect.x),
            top: f64::from(rect.y),
            right: f64::from(rect.right()),
            bottom: f64::from(rect.bottom()),
            width: f64::from(rect.width),
            height: f64::from(rect.height),
        }))
    }

    pub(crate) fn compose_layout_quads_to_top(
        &mut self,
        mut document: DomHandle,
        quads: &mut [moli_layout::LayoutQuad],
    ) -> Result<(), moli_layout::LayoutError> {
        for _ in 0..16 {
            let frame_context = {
                let host = self._context_host.borrow();
                if document == host.document_handle() {
                    return Ok(());
                }
                let Some(frame) = host.child_browsing_context_host_for_document_handle(document)
                else {
                    return Err(moli_layout::LayoutError::source_contract(
                        "frame geometry composition",
                        "child document has no live frame owner",
                    ));
                };
                let Some(parent_document) = host.dom_host().owner_document_handle(frame) else {
                    return Err(moli_layout::LayoutError::source_contract(
                        "frame geometry composition",
                        "frame owner has no parent document",
                    ));
                };
                (
                    frame,
                    parent_document,
                    host.layout_viewport_for_document(document),
                )
            };
            let (frame, parent_document, child_viewport) = frame_context;
            let answers = self.observable_geometry_batch_for_document(
                parent_document,
                moli_layout::LayoutFlushReason::CdpGeometry,
                &moli_layout::LayoutQueryBatch::new(vec![moli_layout::LayoutQuery::BoxModel {
                    source: frame,
                }]),
            )?;
            let Some(moli_layout::LayoutQueryAnswer::BoxModel(Some(frame_model))) =
                answers.answers.into_iter().next()
            else {
                return Err(moli_layout::LayoutError::source_contract(
                    "frame geometry composition",
                    "frame owner has no content-box geometry",
                ));
            };
            for quad in quads.iter_mut() {
                for point in &mut quad.points {
                    *point = map_child_viewport_point_to_parent_content(
                        *point,
                        child_viewport,
                        frame_model.content,
                    );
                }
            }
            document = parent_document;
        }
        Err(moli_layout::LayoutError::source_contract(
            "frame geometry composition",
            "child-frame nesting exceeds the supported depth",
        ))
    }

    pub(crate) fn navigate_child_browsing_context_frame_to_url(
        &mut self,
        frame_id: &str,
        url: &str,
    ) -> Result<bool> {
        let Some(child_handle) = self
            ._context_host
            .borrow()
            .child_browsing_context_handle_by_frame_id(frame_id)
        else {
            return Ok(false);
        };
        self.with_default_context_scope(|scope, host_ptr| {
            Ok(
                unsafe { &mut *host_ptr }.navigate_child_browsing_context_to_url(
                    scope,
                    child_handle,
                    url,
                ),
            )
        })
    }

    pub(crate) fn navigate_top_level_same_document_from_browser(
        &mut self,
        url: &str,
    ) -> Result<bool> {
        let url = url.to_owned();
        self.with_default_context_scope(|scope, _host_ptr| {
            Ok(crate::context_bootstrap::navigate_top_level_same_document_from_browser(scope, url))
        })
    }

    pub(super) fn set_document_ready_state(
        &mut self,
        state: crate::dom::native::DocumentReadyState,
    ) -> Result<()> {
        self.document_runtime.set_document_ready_state(state);
        Ok(())
    }

    pub(super) fn snapshot_live_document(&self) -> NativeDom {
        self.document_runtime.snapshot_document()
    }

    /// Runs late custom-element upgrades at parser/runtime checkpoints.
    ///
    /// Parser-created custom elements whose definitions are known at token creation time are
    /// constructed through the step-scoped `ParserElementCreationConsumer` callback from
    /// `TreeSink::create_element`. This checkpoint walk is only for elements that become
    /// upgradable later, for example because a
    /// parser-blocking script registered their definition after the parser had already created the
    /// element. Fixtures like `connected_from_parser.html` and `legacy/html/slot.html` rely on
    /// those late-definition upgrades being visible before the next parser-connected script runs.
    ///
    /// This compatibility walk happens just after the parser's normal checkpoint. Because the
    /// delayed upgrade may itself run constructors or lifecycle callbacks, finish those reactions
    /// here before the following parser script. This is a named parser-algorithm checkpoint, not
    /// an implicit property of entering the default V8 context.
    pub(crate) fn upgrade_late_defined_custom_elements_after_parser_checkpoint(
        &mut self,
    ) -> Result<()> {
        self.with_default_context_scope(|scope, host_ptr| {
            let document_handle = unsafe { &*host_ptr }.dom_host().document_handle();
            let _ = custom_elements::upgrade_late_defined_connected_tree_after_parser_sync(
                scope,
                host_ptr,
                document_handle,
            );
            perform_microtask_checkpoint_and_report_pending_promise_rejections(scope);
            Ok(())
        })
    }

    // Parser operations reach ScriptVm only when they must enter the page default
    // V8 context. Runtime DOM identity stays inside DocumentRuntime's active
    // parser step.
    pub(crate) fn create_and_construct_parser_custom_element_direct_in_default_context(
        &mut self,
        document_handle: DomHandle,
        document_has_body: bool,
        local_name: &str,
        namespace: &str,
        prefix: Option<&str>,
        token_attributes: &[Attribute],
        intended_parent: Option<DomHandle>,
    ) -> Result<Option<DomHandle>> {
        let context_host = self._context_host.clone();
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        let document_runtime = &mut self.document_runtime;
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                // SAFETY: `context_ptr` points to `self.page_default_context`, which is kept
                // alive for the duration of this non-escaping closure while the document
                // isolate is exclusively borrowed.
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let host_ptr: *mut JsContextHost = (*context_host).as_ptr();
                Ok(
                    custom_elements::create_and_construct_parser_custom_element_direct_for_document(
                        scope,
                        host_ptr,
                        document_handle,
                        document_has_body,
                        local_name,
                        namespace,
                        prefix,
                        token_attributes,
                        intended_parent,
                        |document_handle, local_name, namespace, prefix| {
                            document_runtime.create_parser_element_for_document_without_attributes_in_live_dom_host(
                                document_handle,
                                local_name,
                                namespace,
                                prefix,
                            )
                        },
                    ),
                )
            })
    }

    pub(crate) fn run_pending_parser_post_step_runtime_work_in_default_context(
        &mut self,
    ) -> Result<()> {
        self.with_default_context_scope(|scope, host_ptr| {
            unsafe { &mut *host_ptr }.run_pending_parser_post_step_runtime_work(scope, host_ptr);
            Ok(())
        })
    }

    #[cfg(test)]
    pub(crate) fn queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(
        &mut self,
        work: crate::document_runtime::ParserPostStepRuntimeWorkForTest,
    ) -> Result<()> {
        self.with_default_context_scope(|scope, host_ptr| {
            let runtime = unsafe { &mut *host_ptr };
            runtime.queue_pending_parser_post_step_runtime_work_for_test(work);
            runtime.run_pending_parser_post_step_runtime_work(scope, host_ptr);
            Ok(())
        })
    }

    /// Rebinds the request-client view of the current committed Document.
    ///
    /// The Document authority is installed before realm bootstrap and remains
    /// stable here. Existing leases keep their captured request client; only
    /// subsequently registered loads observe this replacement transport.
    #[cfg(test)]
    pub(super) fn replace_document_resource_runtime(
        &mut self,
        request_client: &ResourceRequestClient,
    ) -> DocumentResourceLoader {
        let navigator_identity = request_client.browser_identity().clone();
        self.replace_document_resource_runtime_with_navigator_identity(
            request_client,
            &navigator_identity,
        )
    }

    pub(super) fn replace_document_resource_runtime_with_navigator_identity(
        &mut self,
        request_client: &ResourceRequestClient,
        navigator_identity: &moli_browser_profile::BrowserIdentityProfile,
    ) -> DocumentResourceLoader {
        let current = self
            .current_main_document_resource_loader()
            .expect("committed Document must install its resource authority before rebinding");
        let document_loader = current.with_replacement_transport(request_client.clone());
        self._context_host
            .borrow_mut()
            .replace_main_document_resource_transport(&document_loader);
        self.document_runtime
            .set_cookie_store(document_loader.request_client().cookie_store());
        let _ = self.with_default_context_scope(|scope, _| {
            set_window_navigator_identity(scope, navigator_identity)
        });
        document_loader
    }

    pub(super) fn set_document_navigator_identity(
        &mut self,
        navigator_identity: &moli_browser_profile::BrowserIdentityProfile,
    ) {
        let _ = self.with_default_context_scope(|scope, _| {
            set_window_navigator_identity(scope, navigator_identity)
        });
    }

    pub(super) fn set_web_storage_handles(&mut self, handles: &crate::RendererWebStorageHandles) {
        self._context_host
            .borrow_mut()
            .set_web_storage_handles(handles);
    }

    pub(super) fn web_storage_handles(&self) -> crate::RendererWebStorageHandles {
        let host = self._context_host.borrow();
        crate::RendererWebStorageHandles::new(
            host.web_storage_store(),
            host.session_storage_store(),
        )
    }

    pub(super) fn set_wpt_extensions_enabled(&mut self, enabled: bool) -> Result<()> {
        self._context_host
            .borrow_mut()
            .set_wpt_extensions_enabled(enabled);
        if !enabled {
            return Ok(());
        }

        #[cfg(not(feature = "wpt-extensions"))]
        {
            Ok(())
        }

        #[cfg(feature = "wpt-extensions")]
        self.with_default_context_scope(|scope, _| {
            let global = scope.get_current_context().global(scope);
            crate::context_bootstrap::install_wpt_webdriver_runtime_state(scope, global)
        })
    }

    pub(crate) fn restore_top_level_location_runtime_state(&mut self, url: &Url) {
        let href = url.as_str().to_owned();
        self.sync_top_level_location_runtime_state(&href);
        if let Some((previous_target, next_target)) =
            self.document_runtime.set_document_url(url.clone())
        {
            self._context_host
                .borrow_mut()
                .note_target_style_activity(previous_target, next_target);
        }
    }

    fn same_document_fragment_url(current: &Url, candidate: &Url) -> bool {
        let mut current_without_fragment = current.clone();
        current_without_fragment.set_fragment(None);
        let mut candidate_without_fragment = candidate.clone();
        candidate_without_fragment.set_fragment(None);
        current_without_fragment == candidate_without_fragment
    }

    fn top_level_context_ptrs(&self) -> Vec<*const v8::Global<v8::Context>> {
        let mut contexts = vec![&self.page_default_context as *const _];
        contexts.extend(
            self.page_isolated_world_contexts
                .contexts()
                .filter(|world| world.child_handle.is_none())
                .map(|world| &world.context as *const _),
        );
        contexts
    }

    fn runtime_binding_replay_context_ptrs(&self) -> Vec<*const v8::Global<v8::Context>> {
        let mut contexts = Vec::with_capacity(
            1 + self.page_isolated_world_contexts.len() + self.child_frame_realm_store.len(),
        );
        contexts.push(&self.page_default_context as *const _);
        contexts.extend(
            self.page_isolated_world_contexts
                .contexts()
                .map(|world| &world.context as *const _),
        );
        contexts.extend(
            self.child_frame_realm_store
                .values()
                .map(|world| &world.context as *const _),
        );
        contexts
    }

    pub(super) fn refresh_top_level_document_url_from_world_locations(&mut self) {
        let current = self.document_runtime.document_url().clone();
        let mut candidates = Vec::new();
        for context_ptr in self.top_level_context_ptrs() {
            // Internal location reconciliation, not an owner-visible script turn.
            let Ok(raw_href) = self.eval_string_in_context_ptr_internal_snapshot(
                context_ptr,
                "(() => String((globalThis.location && globalThis.location.href) || ''))()",
            ) else {
                continue;
            };
            let Ok(candidate) = Url::parse(&raw_href) else {
                continue;
            };
            candidates.push(candidate);
        }

        if candidates
            .iter()
            .any(|candidate| candidate.as_str() == current.as_str())
        {
            self.sync_top_level_location_runtime_state(current.as_str());
            return;
        }

        for candidate in candidates {
            if candidate.as_str() == current.as_str()
                || !Self::same_document_fragment_url(&current, &candidate)
            {
                continue;
            }
            if let Some((previous_target, next_target)) =
                self.document_runtime.set_document_url(candidate.clone())
            {
                self._context_host
                    .borrow_mut()
                    .note_target_style_activity(previous_target, next_target);
            }
            self.sync_top_level_location_runtime_state(candidate.as_str());
            break;
        }
    }

    fn sync_top_level_location_runtime_state(&mut self, href: &str) {
        let href = href.to_owned();
        for context_ptr in self.top_level_context_ptrs() {
            let href = href.clone();
            let _ = self
                .renderer_document_isolate
                .with_entered_renderer_document_isolate(|isolate| {
                    let scope = pin!(v8::HandleScope::new(isolate));
                    let scope = &mut scope.init();
                    let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                    let scope = &mut v8::ContextScope::new(scope, context);
                    sync_global_location_runtime_state(scope, &href);
                    Ok(())
                });
        }
    }

    pub(super) fn retire_document_resource_authorities(&mut self) {
        self._context_host
            .borrow_mut()
            .retire_all_document_resource_loaders();
        self.document_runtime.clear_cookie_store();
    }

    pub(super) fn set_extra_http_headers(&mut self, headers: &[(String, String)]) {
        self._context_host
            .borrow_mut()
            .set_extra_http_headers(headers);
    }

    pub(super) fn set_main_navigation_policy_container(
        &mut self,
        policy: crate::document_runtime::DocumentPolicyContainer,
    ) {
        self.document_runtime
            .set_main_navigation_policy_container(policy);
    }

    pub(super) fn document_content_security_policies(&self) -> Vec<String> {
        self._context_host
            .borrow()
            .document_content_security_policies()
            .to_vec()
    }

    pub(super) fn set_stored_document_start_scripts(&mut self, scripts: &[DocumentStartScript]) {
        self._context_host
            .borrow_mut()
            .set_stored_document_start_scripts(scripts);
    }

    pub(super) fn set_stored_runtime_bindings(
        &mut self,
        bindings: &[crate::protocol_types::RuntimeBindingRegistration],
    ) {
        self._context_host
            .borrow_mut()
            .set_stored_runtime_bindings(bindings);
    }

    pub(super) fn set_inspector_session_runtime_bindings(
        &mut self,
        inspector_session_id: Option<&str>,
        bindings: &[crate::protocol_types::RuntimeBindingRegistration],
    ) {
        self.page_inspector
            .set_runtime_bindings_for_session(inspector_session_id, bindings);
    }

    pub(super) fn inspector_session_runtime_bindings(
        &self,
        inspector_session_id: Option<&str>,
    ) -> Vec<crate::protocol_types::RuntimeBindingRegistration> {
        self.page_inspector
            .runtime_bindings_for_session(inspector_session_id)
    }

    pub(super) fn detach_runtime_inspector_session(
        &mut self,
        inspector_session_id: Option<&str>,
    ) -> bool {
        self._context_host
            .borrow_mut()
            .remove_dom_debugger_session(inspector_session_id);
        let devtools_session = DevToolsSessionKey::from_wire_session_id(
            inspector_session_id.filter(|session_id| !session_id.is_empty()),
        );
        let detached = self.page_inspector.detach_session(inspector_session_id);
        let retired_worlds = self.retire_isolated_worlds_for_devtools_session(&devtools_session);
        detached || retired_worlds != 0
    }

    pub(super) fn set_permission_overrides(
        &mut self,
        overrides: &[crate::protocol_types::PermissionOverrideRegistration],
    ) {
        self._context_host
            .borrow_mut()
            .set_permission_overrides(overrides);
    }

    pub(super) fn set_locale_override(&mut self, locale: Option<&str>) {
        self._context_host.borrow_mut().set_locale_override(locale);
    }

    pub(super) fn set_locale_override_and_sync_surface(
        &mut self,
        locale: Option<&str>,
    ) -> Result<()> {
        self.set_locale_override(locale);
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        self.with_context_scope_by_ptr(context_ptr, |scope, _| {
            set_date_locale_override_for_current_context(scope, locale);
            Ok(())
        })
    }

    pub(super) fn set_timezone_override(&mut self, timezone: Option<&str>) {
        self._context_host
            .borrow_mut()
            .set_timezone_override(timezone);
    }

    pub(super) fn set_timezone_override_and_sync_surface(
        &mut self,
        timezone: Option<&str>,
    ) -> Result<()> {
        self.set_timezone_override(timezone);
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        self.with_context_scope_by_ptr(context_ptr, |scope, _| {
            set_date_timezone_override_for_current_context(scope, timezone);
            Ok(())
        })
    }

    pub(super) fn set_emulated_media(
        &mut self,
        overrides: &crate::protocol_types::EmulatedMediaOverrides,
    ) {
        let (previous_media, viewport) = {
            let host = self._context_host.borrow();
            (host.emulated_media().clone(), host.style_viewport())
        };
        self._context_host
            .borrow_mut()
            .set_emulated_media(overrides);
        self.dispatch_media_query_list_change_events(
            &previous_media,
            viewport,
            overrides,
            viewport,
        );
    }

    pub(super) fn set_idle_override(
        &mut self,
        idle_override: Option<crate::protocol_types::EmulatedIdleOverride>,
    ) {
        self._context_host
            .borrow_mut()
            .set_idle_override(idle_override);
    }

    pub(super) fn set_idle_override_and_sync_surface(
        &mut self,
        idle_override: Option<crate::protocol_types::EmulatedIdleOverride>,
    ) -> Result<()> {
        self.set_idle_override(idle_override);
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        self.with_context_scope_by_ptr(context_ptr, |scope, _| {
            crate::context_bootstrap::apply_idle_override_to_current_context(scope, idle_override);
            Ok(())
        })
    }

    pub(super) fn stylesheet_preload_media_matches(&self, media: Option<&str>) -> bool {
        let Some(media) = media.map(str::trim).filter(|media| !media.is_empty()) else {
            return true;
        };
        let host = self._context_host.borrow();
        crate::style_engine::media_list::evaluate_media_query_list(
            media,
            Some(host.emulated_media()),
            host.style_viewport(),
        )
    }

    pub(super) fn fetch_subresource_interception_matches(
        &self,
        resource_type: crate::types::SubresourceResourceType,
    ) -> bool {
        let host = self._context_host.borrow();
        host.fetch_subresource_interception_enabled()
            && host
                .fetch_subresource_interception_resource_type()
                .is_none_or(|intercepted| intercepted == resource_type)
    }

    pub(super) fn set_viewport_surface(
        &mut self,
        viewport_surface: Option<crate::protocol_types::ViewportSurface>,
    ) -> Result<()> {
        let (previous_media, previous_viewport) = {
            let host = self._context_host.borrow();
            (host.emulated_media().clone(), host.style_viewport())
        };
        let changed = self
            ._context_host
            .borrow_mut()
            .set_viewport_surface(viewport_surface);
        if !changed {
            return Ok(());
        }
        let (current_media, current_viewport) = {
            let host = self._context_host.borrow();
            (host.emulated_media().clone(), host.style_viewport())
        };
        let width = current_viewport
            .width
            .unwrap_or(moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.inner_width);
        let height = current_viewport
            .height
            .unwrap_or(moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.inner_height);
        self.with_default_context_scope(|scope, _host_ptr| {
            let window = scope.get_current_context().global(scope);
            crate::context_bootstrap::update_cached_window_visual_viewport_dimensions(
                scope, window, width, height,
            );
            Ok(())
        })?;
        self.dispatch_media_query_list_change_events(
            &previous_media,
            previous_viewport,
            &current_media,
            current_viewport,
        );
        Ok(())
    }

    /// Seeds the initial viewport before document-start scripts can materialize
    /// Window surfaces or register media-query listeners.
    pub(super) fn set_viewport_surface_for_bootstrap(
        &mut self,
        viewport_surface: Option<crate::protocol_types::ViewportSurface>,
    ) {
        self._context_host
            .borrow_mut()
            .set_viewport_surface(viewport_surface);
    }

    pub(super) fn set_layout_policy(&mut self, policy: moli_page_types::LayoutPolicy) {
        self._context_host.borrow_mut().set_layout_policy(policy);
    }

    #[cfg(test)]
    pub(crate) fn force_fresh_layout_reads_for_test(&mut self) {
        self._context_host
            .borrow_mut()
            .force_fresh_layout_reads_for_test();
    }

    fn dispatch_media_query_list_change_events(
        &mut self,
        previous_media: &crate::protocol_types::EmulatedMediaOverrides,
        previous_viewport: crate::style_engine::StyleViewport,
        current_media: &crate::protocol_types::EmulatedMediaOverrides,
        current_viewport: crate::style_engine::StyleViewport,
    ) {
        self.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, &self.page_default_context);
                let scope = &mut v8::ContextScope::new(scope, context);
                dispatch_media_query_list_change_events_for_scope(
                    scope,
                    previous_media,
                    previous_viewport,
                    current_media,
                    current_viewport,
                );
                let host = self._context_host.borrow();
                for document in host.documents_with_adopted_style_sheets() {
                    crate::native_bridge::document::sync_document_fonts_for_handle(
                        scope, &host, document,
                    );
                }
            });
    }

    pub(super) fn set_network_offline(&mut self, offline: bool) {
        self._context_host.borrow_mut().set_network_offline(offline);
    }

    pub(super) fn set_bypass_service_worker(&mut self, bypass: bool) {
        self._context_host
            .borrow_mut()
            .set_bypass_service_worker(bypass);
    }

    pub(super) fn set_cache_disabled(&mut self, disabled: bool) {
        self._context_host.borrow_mut().set_cache_disabled(disabled);
    }

    pub(super) fn set_blocked_url_patterns(&mut self, patterns: &[String]) {
        self._context_host
            .borrow_mut()
            .set_blocked_url_patterns(patterns);
    }

    pub(super) fn set_fetch_subresource_interception(
        &mut self,
        enabled: bool,
        resource_type: Option<super::SubresourceResourceType>,
    ) {
        self._context_host
            .borrow_mut()
            .set_fetch_subresource_interception(enabled, resource_type);
    }

    pub(super) fn resync_child_browsing_contexts(&mut self) {
        child_host_load::ChildHostLoadOwner::new(self).resync_child_browsing_contexts();
    }

    #[cfg(test)]
    pub(super) async fn run_child_frame_task_source_once_for_test(
        &mut self,
        turn: impl Into<crate::frame_owner_model::ChildFrameSemanticTurnKind>,
    ) -> bool {
        use crate::frame_owner_model::ChildFrameSemanticTurnKind;

        match turn.into() {
            ChildFrameSemanticTurnKind::RealmMaterialization => self
                .run_child_realm_materialization_body_for_test()
                .expect("typed child realm-materialization executor turn should succeed"),
            ChildFrameSemanticTurnKind::DocumentScriptReady => self
                .run_child_document_script_ready_body_for_test()
                .await
                .expect("typed child DocumentScriptReady executor turn should succeed")
                .is_some_and(ChildDocumentScriptReadyRunOutcome::made_progress),
            ChildFrameSemanticTurnKind::NavigationCommit => self
                .run_next_child_navigation_commit_body_for_test()
                .expect("typed child navigation-commit body should succeed")
                .is_some(),
            ChildFrameSemanticTurnKind::DocumentLifecycle => self
                .run_child_document_lifecycle_body_for_test()
                .expect("typed child lifecycle executor turn should succeed")
                .is_some(),
            ChildFrameSemanticTurnKind::HostLoad => self
                .run_child_host_load_body_for_test()
                .expect("typed child HostLoad executor turn should succeed")
                .is_some(),
            ChildFrameSemanticTurnKind::ClassicScriptSourceLoad => self
                .run_child_classic_source_load_body_for_test()
                .expect("typed child classic source-load body should succeed")
                .is_some(),
            ChildFrameSemanticTurnKind::ParserModuleRootStart => self
                .run_child_parser_module_root_start_body_for_test()
                .expect("typed child parser module root body should succeed")
                .is_some(),
        }
    }

    #[cfg(test)]
    pub(super) fn has_ready_child_frame_semantic_turn_for_test(
        &self,
        expected: crate::frame_owner_model::ChildFrameSemanticTurnKind,
    ) -> bool {
        use crate::{
            frame_owner_model::ChildFrameSemanticTurnKind,
            page_task_queue::RendererPageChildFrameTaskTarget,
        };

        let Some(target) = self
            ._page_task_residence_for_executor_test
            .as_ref()
            .and_then(|residence| residence.task_sources().next_child_frame_task_target())
        else {
            return false;
        };
        matches!(
            (expected, target),
            (
                ChildFrameSemanticTurnKind::RealmMaterialization,
                RendererPageChildFrameTaskTarget::RealmMaterialization(_)
            ) | (
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                RendererPageChildFrameTaskTarget::DocumentLifecycle(_)
            ) | (
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                RendererPageChildFrameTaskTarget::DocumentScriptReady(_)
            ) | (
                ChildFrameSemanticTurnKind::HostLoad,
                RendererPageChildFrameTaskTarget::HostLoad(_)
            ) | (
                ChildFrameSemanticTurnKind::ClassicScriptSourceLoad,
                RendererPageChildFrameTaskTarget::ClassicScriptSourceLoad(_)
            ) | (
                ChildFrameSemanticTurnKind::ParserModuleRootStart,
                RendererPageChildFrameTaskTarget::ParserModuleRootStart(_)
            )
        )
    }

    /// Run one child semantic executor turn from the production stable
    /// residences used by low-level ScriptVm fixtures.
    #[cfg(test)]
    pub(super) async fn run_next_child_frame_semantic_turn_for_test(
        &mut self,
    ) -> Option<crate::frame_owner_model::ChildFrameSemanticTurnKind> {
        use crate::frame_owner_model::ChildFrameSemanticTurnKind;

        if self
            .run_child_realm_materialization_body_for_test()
            .expect("child realm materialization prerequisite should succeed")
        {
            return Some(ChildFrameSemanticTurnKind::RealmMaterialization);
        }
        if self
            .run_next_child_navigation_commit_body_for_test()
            .expect("typed child navigation-commit body should succeed")
            .is_some()
        {
            return Some(ChildFrameSemanticTurnKind::NavigationCommit);
        }
        if self
            .run_child_document_lifecycle_body_for_test()
            .expect("typed child lifecycle executor turn should succeed")
            .is_some()
        {
            return Some(ChildFrameSemanticTurnKind::DocumentLifecycle);
        }
        if self
            .run_child_document_script_ready_body_for_test()
            .await
            .expect("typed child DocumentScriptReady executor turn should succeed")
            .is_some()
        {
            return Some(ChildFrameSemanticTurnKind::DocumentScriptReady);
        }
        if self
            .run_child_host_load_body_for_test()
            .expect("typed child HostLoad executor turn should succeed")
            .is_some()
        {
            return Some(ChildFrameSemanticTurnKind::HostLoad);
        }
        if self
            .run_child_parser_module_root_start_body_for_test()
            .expect("typed child parser module root body should succeed")
            .is_some()
        {
            return Some(ChildFrameSemanticTurnKind::ParserModuleRootStart);
        }
        if self
            .run_child_classic_source_load_body_for_test()
            .expect("typed child classic source-load body should succeed")
            .is_some()
        {
            return Some(ChildFrameSemanticTurnKind::ClassicScriptSourceLoad);
        }
        None
    }

    fn apply_child_parser_module_root_fetch_completion_to_owner(
        &mut self,
        completion: crate::types::ChildParserModuleRootFetchCompletion,
    ) -> crate::frame_owner_model::FrameDocumentModuleTerminalQueueFollowup {
        child_module_fetch::ChildModuleFetchOwner::new(self)
            .apply_parser_root_fetch_completion(completion)
    }

    fn apply_child_module_dependency_fetch_completion_to_owner(
        &mut self,
        completion: crate::types::ChildModuleDependencyFetchCompletion,
    ) -> crate::frame_owner_model::FrameDocumentModuleTerminalQueueFollowup {
        child_module_fetch::ChildModuleFetchOwner::new(self)
            .apply_dependency_fetch_completion(completion)
    }

    #[cfg(test)]
    pub(super) fn drain_pending_child_frame_work_for_test(&mut self) {
        expect_ready_child_frame_owner_source_future_for_test(
            self.drain_pending_child_frame_work_for_test_on_owner_sources(),
        );
    }

    #[cfg(test)]
    async fn drain_pending_child_frame_work_for_test_on_owner_sources(&mut self) {
        // Direct ScriptVm semantic tests have no owner loop, so advance the
        // same production sources one turn at a time. Never jump a dependent
        // child action over its realm prerequisite in the stable family FIFO.
        for _ in 0..128 {
            if self
                .run_child_realm_materialization_body_for_test()
                .expect("test child realm owner turn should complete")
            {
                continue;
            }
            if self
                .run_next_child_navigation_commit_body_for_test()
                .expect("test child navigation-commit body should complete")
                .is_some()
            {
                continue;
            }
            if self
                .run_child_document_lifecycle_body_for_test()
                .expect("test child lifecycle owner turn should complete")
                .is_some()
            {
                continue;
            }
            if self
                .run_child_document_script_ready_body_for_test()
                .await
                .expect("test child script owner turn should complete")
                .is_some()
            {
                continue;
            }
            if self
                .run_child_host_load_body_for_test()
                .expect("test child HostLoad owner turn should complete")
                .is_some()
            {
                continue;
            }
            if self
                .run_child_parser_module_root_start_body_for_test()
                .expect("test child parser-root body should complete")
                .is_some()
            {
                continue;
            }
            if self
                .run_child_classic_source_load_body_for_test()
                .expect("test child classic source body should complete")
                .is_some()
            {
                continue;
            }
            if self.run_child_module_script_terminal_body_for_test() {
                continue;
            }
            return;
        }
        panic!("test child owner-source drain exceeded its finite turn budget");
    }

    pub(super) fn has_pending_child_document_lifecycle(&self) -> bool {
        self._context_host
            .borrow()
            .has_pending_child_document_lifecycle()
    }

    #[cfg(test)]
    pub(super) fn has_pending_lightweight_popup_document_loads(&self) -> bool {
        self._context_host
            .borrow()
            .has_pending_lightweight_popup_document_loads()
    }

    pub(super) fn has_pending_lightweight_popup_resource_loads(&self) -> bool {
        self._context_host
            .borrow()
            .has_pending_lightweight_popup_resource_loads()
    }

    fn sync_child_browsing_context_records(&mut self) {
        self.resync_child_browsing_contexts();
        self.apply_pending_child_document_owner_retirements();
        self.prune_stale_child_default_execution_contexts();
    }

    #[cfg(test)]
    pub(super) fn has_pending_child_navigation_commit_for_test(&self) -> bool {
        self._context_host
            .borrow()
            .has_pending_child_navigation_commit_task()
    }

    pub(super) fn has_pending_location_navigation(&self) -> bool {
        self._context_host
            .borrow()
            .has_pending_location_navigation()
    }

    pub(super) fn pending_location_navigation_kind(
        &self,
    ) -> Option<super::native_bridge::PendingLocationNavigationKind> {
        self._context_host
            .borrow()
            .pending_location_navigation_kind()
    }

    pub(super) fn has_pending_javascript_location_navigation(&self) -> bool {
        self.pending_location_navigation_kind()
            == Some(super::native_bridge::PendingLocationNavigationKind::JavascriptUrl)
    }

    pub(super) fn pending_location_navigation_runtime_command_cause(
        &self,
    ) -> Option<crate::runtime::RendererRuntimeCommandCausalIdentity> {
        self._context_host
            .borrow()
            .pending_location_navigation_runtime_command_cause()
    }

    pub(super) fn pending_location_navigation_handoff(
        &self,
    ) -> Option<crate::page_task_queue::RendererTopLevelNavigationHandoff> {
        self._context_host
            .borrow()
            .pending_location_navigation_handoff()
    }

    pub(super) fn child_browsing_context_frame_tree_snapshot(
        &mut self,
    ) -> Vec<super::native_bridge::ChildBrowsingContextFrameSnapshot> {
        self._context_host
            .borrow_mut()
            .child_browsing_context_frame_tree_snapshot()
    }

    pub(super) fn top_document_storage_key_snapshot(&mut self) -> String {
        self._context_host
            .borrow_mut()
            .top_document_storage_context()
            .storage_key()
            .serialized_storage_key()
    }

    pub(super) fn child_browsing_context_frame_tree_snapshot_for_protocol(
        &mut self,
    ) -> Vec<crate::protocol_types::ChildFrameTreeSnapshot> {
        fn convert(
            snapshot: super::native_bridge::ChildBrowsingContextFrameSnapshot,
        ) -> crate::protocol_types::ChildFrameTreeSnapshot {
            crate::protocol_types::ChildFrameTreeSnapshot {
                frame_id: snapshot.frame_id,
                loader_id: snapshot.loader_id,
                name: snapshot.name,
                owner_element_id: snapshot.owner_element_id,
                url: snapshot.url,
                storage_key: snapshot.storage_key,
                security_origin_inherited: snapshot.security_origin_inherited,
                security_origin_opaque: snapshot.security_origin_opaque,
                child_frames: snapshot.child_frames.into_iter().map(convert).collect(),
            }
        }

        self.child_browsing_context_frame_tree_snapshot()
            .into_iter()
            .map(convert)
            .collect()
    }

    pub(super) fn child_browsing_context_owner_node_id_by_frame_id(
        &self,
        frame_id: &str,
    ) -> Option<crate::dom::NodeId> {
        self._context_host
            .borrow()
            .child_browsing_context_owner_node_id_by_frame_id(frame_id)
    }

    pub(super) fn child_browsing_context_frame_id_by_owner_node_id(
        &self,
        owner_node_id: crate::dom::NodeId,
    ) -> Option<String> {
        self._context_host
            .borrow()
            .child_browsing_context_frame_id_by_owner_node_id(owner_node_id)
    }

    pub(super) fn child_browsing_context_is_same_origin_with_top(
        &self,
        owner_node_id: crate::dom::NodeId,
    ) -> bool {
        self._context_host
            .borrow()
            .child_browsing_context_is_same_origin_with_top(owner_node_id)
    }

    pub(super) fn child_browsing_context_has_opaque_origin(
        &self,
        owner_node_id: crate::dom::NodeId,
    ) -> bool {
        self._context_host
            .borrow()
            .child_browsing_context_has_opaque_origin(owner_node_id)
    }

    pub(super) fn child_browsing_context_current_url(
        &self,
        owner_node_id: crate::dom::NodeId,
    ) -> Option<url::Url> {
        self._context_host
            .borrow()
            .child_browsing_context_current_url(owner_node_id)
    }

    pub(super) fn child_browsing_context_parent_frame_id(
        &self,
        owner_node_id: crate::dom::NodeId,
    ) -> Option<String> {
        self._context_host
            .borrow()
            .child_browsing_context_parent_frame_id(owner_node_id)
    }

    pub(super) fn child_browsing_context_document_handle_by_frame_id(
        &self,
        frame_id: &str,
    ) -> Option<crate::dom::NodeId> {
        let host = self._context_host.borrow();
        let child_handle = host.child_browsing_context_owner_node_id_by_frame_id(frame_id)?;
        host.child_browsing_context_document_handle(child_handle)
    }

    pub(super) fn live_child_document_handles_in_snapshot_order(
        &self,
    ) -> Vec<(String, crate::dom::NodeId, crate::dom::NodeId)> {
        let host = self._context_host.borrow();
        host.child_browsing_context_handles_in_document_order()
            .into_iter()
            .filter_map(|child_handle| {
                let frame_id = host.frame_owner_frame_id_for_child_handle(child_handle)?.0;
                let document_handle = host.child_browsing_context_document_handle(child_handle)?;
                Some((frame_id, child_handle, document_handle))
            })
            .collect()
    }

    pub(super) fn detached_child_browsing_context_document_snapshots_for_dom_snapshot(
        &mut self,
        top_frame_id: &str,
    ) -> Vec<super::native_bridge::DetachedChildBrowsingContextDocumentSnapshot> {
        self._context_host
            .borrow_mut()
            .detached_child_browsing_context_document_snapshots_for_dom_snapshot(top_frame_id)
    }

    pub(super) fn child_browsing_context_document_snapshot_by_frame_id(
        &mut self,
        frame_id: &str,
    ) -> Option<super::native_bridge::ChildBrowsingContextDocumentSnapshot> {
        self._context_host
            .borrow_mut()
            .child_browsing_context_document_snapshot_by_frame_id(frame_id)
    }

    pub(super) fn take_pending_location_navigation_with_seed(
        &mut self,
    ) -> Option<super::native_bridge::PendingLocationNavigation> {
        self._context_host
            .borrow_mut()
            .take_pending_location_navigation()
    }

    pub(super) fn take_pending_location_navigation_of_kind(
        &mut self,
        kind: super::native_bridge::PendingLocationNavigationKind,
    ) -> Option<super::native_bridge::PendingLocationNavigation> {
        self._context_host
            .borrow_mut()
            .take_pending_location_navigation_of_kind(kind)
    }

    pub(super) fn take_pending_document_location_navigation(
        &mut self,
    ) -> Option<super::native_bridge::PendingLocationNavigation> {
        let source_url = self.document_runtime.document_url().clone();
        let pending = self.take_pending_location_navigation_of_kind(
            super::native_bridge::PendingLocationNavigationKind::Document,
        )?;
        self.restore_top_level_location_runtime_state(&source_url);
        Some(pending)
    }

    /// Moves one browser-owned location request into the active concrete
    /// renderer output sink.
    ///
    /// The request remains renderer-local until lifecycle/command arbitration
    /// proves that the browser owns the navigation. At that exact boundary we
    /// consume it once, freeze its source Document and command cause, and stop
    /// relying on protocol to pull mutable Page state in a later turn.
    pub(super) fn publish_pending_document_location_navigation(&mut self) -> anyhow::Result<bool> {
        let Some(pending) = self.take_pending_document_location_navigation() else {
            return Ok(false);
        };
        let source_document = pending.source_document.ok_or_else(|| {
            anyhow::anyhow!(
                "Page-owned location navigation was produced without an exact source Document"
            )
        })?;
        let runtime_command_cause = pending.runtime_command_cause;
        let action = crate::runtime::RendererOwnerAction::TopLevelLocationNavigation(
            crate::runtime::RendererDocumentSourcedTopLevelLocationNavigation::
                new_with_request_and_runtime_command_cause(
                    source_document,
                    pending.url.to_string(),
                    pending.request_method,
                    pending.request_body,
                    pending.request_headers,
                    pending.browser_navigation_kind,
                    runtime_command_cause.clone(),
                ),
        );
        anyhow::ensure!(
            self._context_host
                .borrow()
                .append_owner_action_with_cause(runtime_command_cause, action),
            "browser-owned location navigation requires an active renderer output sink"
        );
        Ok(true)
    }

    #[cfg(test)]
    pub(super) fn take_pending_top_level_history_traversal(
        &mut self,
    ) -> Option<crate::runtime::RendererPendingTopLevelHistoryTraversal> {
        self._context_host
            .borrow_mut()
            .take_pending_top_level_history_traversal()
    }

    pub(super) fn prepare_top_level_meta_refresh_navigation(
        &mut self,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> Option<MainDocumentLifecycleFollowup> {
        if self.has_pending_location_navigation()
            || !self
                ._context_host
                .borrow()
                .current_main_document_load_has_dispatched(owner)
        {
            return None;
        }
        let scheduled = self
            .document_runtime
            .finish_top_level_meta_refresh_load(owner)?;
        let delay_ms = scheduled.navigation.delay_ms;
        let url = scheduled.navigation.url.clone();
        let ready_at = scheduled.ready_at;
        debug_assert_eq!(scheduled.owner, owner);
        tracing::debug!(?owner, %url, delay_ms, ?ready_at, "prepared post-load top-level meta refresh task");
        let (task, ready_at) = scheduled.into_internal_loading_task();
        Some(MainDocumentLifecycleFollowup::ScheduleInternalLoading { task, ready_at })
    }

    pub(super) fn schedule_page_internal_loading_task(
        &self,
        task: crate::page_task_queue::PageOwnedInternalLoadingTask,
        ready_at: Instant,
    ) -> anyhow::Result<()> {
        self._context_host
            .borrow()
            .page_internal_loading_sender()
            .schedule_at(task, ready_at)
            .map_err(|_| anyhow::anyhow!("live Page internal-loading source closed"))
    }

    pub(super) fn run_page_owned_internal_loading_task(
        &mut self,
        task: crate::page_task_queue::PageOwnedInternalLoadingTask,
    ) -> crate::page_task_queue::PageOwnedInternalLoadingTaskEffect {
        let crate::page_task_queue::PageOwnedInternalLoadingTask::MetaRefreshNavigation(task) =
            task;
        let owner = task.owner();
        let delay_ms = task.delay_ms();
        let url = task.into_url();
        let scheduler_owned_task = self
            .document_runtime
            .consume_top_level_meta_refresh_navigation(owner, delay_ms, &url);
        if !scheduler_owned_task {
            tracing::debug!(
                ?owner,
                %url,
                "discarded top-level meta refresh because its scheduler ownership no longer matches"
            );
            return crate::page_task_queue::PageOwnedInternalLoadingTaskEffect::MetaRefreshNavigationNotActivated;
        }
        if self.has_pending_location_navigation() {
            // Consumption deliberately happens before this check. A competing
            // navigation that has already started supersedes the refresh, so
            // the posted task is retired rather than retried against a later
            // Document or after the competing navigation is handed off.
            tracing::debug!(
                ?owner,
                %url,
                "retired top-level meta refresh because a competing navigation is already pending"
            );
            return crate::page_task_queue::PageOwnedInternalLoadingTaskEffect::MetaRefreshNavigationNotActivated;
        }
        tracing::debug!(?owner, %url, delay_ms, "activating post-load top-level meta refresh navigation");
        let activated = self
            .with_default_context_scope(|scope, _host_ptr| {
                Ok(crate::context_bootstrap::navigate_top_level_meta_refresh(
                    scope, &url, delay_ms,
                ))
            })
            .unwrap_or(false);
        if activated {
            crate::page_task_queue::PageOwnedInternalLoadingTaskEffect::MetaRefreshNavigationActivated
        } else {
            crate::page_task_queue::PageOwnedInternalLoadingTaskEffect::MetaRefreshNavigationNotActivated
        }
    }

    pub(crate) fn apply_parser_stream_mutation_effects_to_live_dom_host_in_default_context(
        &mut self,
        effects: DomMutationEffects,
    ) -> Result<()> {
        self.with_default_context_scope(|scope, host_ptr| {
            unsafe { &mut *host_ptr }
                .apply_parser_stream_mutation_effects_to_live_dom_host(scope, host_ptr, effects);
            Ok(())
        })
    }

    pub(crate) fn apply_parser_dom_mutation_to_live_dom_host_in_default_context(
        &mut self,
        mutation: crate::parser::ParserDomMutation,
    ) -> Result<()> {
        self.with_default_context_scope(|scope, host_ptr| {
            unsafe { &mut *host_ptr }
                .apply_parser_dom_mutation_to_live_dom_host(scope, host_ptr, mutation);
            Ok(())
        })
    }

    pub(crate) fn apply_parser_created_null_registry_associations_in_default_context(
        &mut self,
        handles: &[NativeNodeId],
    ) -> Result<()> {
        self.with_default_context_scope(|_scope, host_ptr| {
            custom_elements::apply_parser_created_null_registry_associations(host_ptr, handles);
            Ok(())
        })
    }

    pub(crate) fn construct_parser_custom_element_handoff(
        &mut self,
        handoff: &crate::parser::ParserCustomElementConstructionHandoff,
    ) -> Result<()> {
        self.with_default_context_scope(|scope, host_ptr| {
            let _ = custom_elements::construct_parser_created_autonomous_element_from_handoff(
                scope, host_ptr, handoff,
            );
            Ok(())
        })
    }

    pub(crate) fn flush_parser_custom_element_handoff_replacements(&mut self) -> Result<()> {
        self.with_default_context_scope(|scope, host_ptr| {
            custom_elements::flush_parser_custom_element_handoff_replacements(scope, host_ptr);
            Ok(())
        })
    }

    pub(super) fn install_navigation_bootstrap_entry(
        &mut self,
        entry_seed: Option<super::native_bridge::NavigationHistoryEntrySeed>,
    ) {
        let Some(entry_seed) = entry_seed else {
            return;
        };
        let _ = self.with_default_context_scope(|scope, _runtime_ptr| {
            super::context_bootstrap::install_navigation_bootstrap_entry(scope, &entry_seed);
            Ok(())
        });
    }

    #[cfg(test)]
    pub(super) async fn advance_timers_until_deadline_for_test(
        &mut self,
        loader: &ResourceRequestClient,
    ) -> Result<()> {
        let deadline = Instant::now()
            .checked_add(std::time::Duration::from_millis(3_200))
            .unwrap_or_else(Instant::now);
        self.advance_timers_until_deadline_for_test_with_deadline(loader, deadline)
            .await
    }

    #[cfg(test)]
    pub(super) async fn advance_timers_until_deadline_for_test_with_deadline(
        &mut self,
        loader: &ResourceRequestClient,
        deadline: Instant,
    ) -> Result<()> {
        const MAX_TEST_ADVANCE_ROUNDS: usize = 10_000;
        let mut rounds = 0usize;

        while rounds < MAX_TEST_ADVANCE_ROUNDS {
            // This is an explicit low-level executor test helper. Production
            // callers must enter through the scheduler-selected PageTimer
            // turn, never through a generic ready-task drain.
            if self.has_ready_timeout() && self.run_next_due_timer_callback_for_test(loader).await?
            {
                rounds += 1;
                continue;
            }

            let Some(ms_to_next) = self.ms_to_next_timeout() else {
                break;
            };
            if ms_to_next == 0 {
                rounds += 1;
                continue;
            }

            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let sleep_for = std::time::Duration::from_millis(ms_to_next)
                .min(deadline.saturating_duration_since(now));
            if sleep_for.is_zero() {
                break;
            }
            tokio::time::sleep(sleep_for).await;
            rounds += 1;
        }

        Ok(())
    }

    /// Execute one due timer task body after the Page scheduler has validated
    /// its observed heap-head deadline.
    ///
    /// This method deliberately does not checkpoint, synchronize child
    /// records, or run runtime follow-up. The selected Page-task dispatcher
    /// commits that completion exactly once after the body returns.
    pub(crate) fn run_next_due_timer_callback_body(
        &mut self,
        selection: crate::page_task_queue::RendererPageTimerSelection,
    ) -> Result<HostTimeoutRunResult> {
        let result = self.run_next_timeout_body(selection)?;
        if let HostTimeoutRunResult::CallbackError(error) = &result {
            self.record_runtime_warning(format_args!("timer callback dispatch failed: {error}"));
        }
        Ok(result)
    }

    /// Complete one timer in a standalone ScriptVm fixture.
    ///
    /// Production and PageVm behavior tests must use the selected Page-task
    /// dispatcher. Standalone domain fixtures have no Page owner slot, so this
    /// helper explicitly supplies the same bounded callback completion.
    #[cfg(test)]
    pub(crate) async fn run_next_due_timer_callback_for_test(
        &mut self,
        loader: &ResourceRequestClient,
    ) -> Result<bool> {
        let result = self.run_next_due_timer_callback_body(
            crate::page_task_queue::RendererPageTimerSelection::AnyReady,
        )?;
        if !result.consumed_heap_head() {
            return Ok(false);
        }
        // Timer turns are ordinary runtime activity. Runtime follow-up may
        // publish concrete Page work, but must not wait for network completion.
        self.finish_selected_page_callback_task(loader).await?;
        Ok(true)
    }

    #[cfg(test)]
    pub(crate) fn has_ready_window_message_task(&self) -> bool {
        self._context_host.borrow().has_pending_window_messages()
    }

    #[cfg(test)]
    pub(crate) fn pending_window_message_endpoints_for_test(
        &self,
    ) -> Vec<(
        crate::native_bridge::PendingWindowMessageEndpoint,
        crate::native_bridge::PendingWindowMessageEndpoint,
    )> {
        self._context_host
            .borrow()
            .pending_window_message_endpoints_for_test()
    }

    pub(super) async fn finish_host_task_turn(
        &mut self,
        loader: &ResourceRequestClient,
        wait_for_dynamic_loads: bool,
    ) -> Result<()> {
        self.flush_pending_work(loader, wait_for_dynamic_loads)
            .await
            .map_err(anyhow::Error::msg)?;
        Ok(())
    }

    pub(super) async fn run_prepared_script(
        &mut self,
        loader: &ResourceRequestClient,
        script: &PreparedScript,
        dynamic_script_owner_id: Option<crate::dynamic_script_owner::DynamicScriptOwnerId>,
    ) -> std::result::Result<PreparedScriptExecutionOutcome, PreparedScriptExecutionError> {
        debug!(
            url = %script.url,
            mode = ?script.mode,
            kind = ?script.kind,
            "run_prepared_script begin"
        );
        let Some(run_input) = self.prepare_prepared_script_run(loader, script).await? else {
            return Ok(PreparedScriptExecutionOutcome::Dropped(
                PreparedScriptBodyActivity::NotEntered,
            ));
        };
        self.document_runtime
            .set_current_script_context(CurrentScriptContextSpec {
                handle: run_input.current_script,
                parser_write_insertion_point_active: run_input.parser_write_insertion_point_active,
                parser_bridge: None,
            });
        let document_owner_before_run =
            self.current_main_document_task_owner().ok_or_else(|| {
                PreparedScriptExecutionError::from_message(format!(
                    "prepared script `{}` has no current main Document owner",
                    script.url
                ))
            })?;
        let result = self
            .execute_prepared_script_run_body(script, run_input.body)
            .await;
        self.document_runtime.clear_current_script_handle();
        let outcome = result?;
        let body_activity = outcome.body_activity();
        if self.script_run_replaced_document(document_owner_before_run, script) {
            return Ok(PreparedScriptExecutionOutcome::Dropped(body_activity));
        }
        match outcome {
            LoadedScriptExecutionOutcome::Completed(_) => {}
            LoadedScriptExecutionOutcome::CompletedModuleGraph(graph) => {
                let continuation = self
                    .module_script_continuation_for_prepared_script(
                        script,
                        dynamic_script_owner_id,
                        document_owner_before_run,
                    )?
                    .with_completed_graph(graph);
                let actions = self.handle_module_script_graph_advance_for_owner(
                    ModuleScriptContinuationGraphAdvance::Ready(Box::new(continuation)),
                );
                debug_assert!(
                    actions.into_runtime_module_failures().is_empty(),
                    "completed module graph should only enqueue owner-ready work"
                );
                return Ok(PreparedScriptExecutionOutcome::DeferredModuleCompletion);
            }
            LoadedScriptExecutionOutcome::SuspendedModuleFetches(fetches) => {
                let (job, fetches) = fetches.into_parts();
                let continuation = self.module_script_continuation_for_prepared_script(
                    script,
                    dynamic_script_owner_id,
                    document_owner_before_run,
                )?;
                let actions = self.handle_module_script_graph_advance_for_owner(
                    ModuleScriptContinuationGraphAdvance::NeedFetches {
                        continuation: Box::new(continuation),
                        job: Box::new(job),
                        fetches,
                    },
                );
                debug_assert!(
                    actions.into_runtime_module_failures().is_empty(),
                    "suspended module graph should only enqueue owner wait work"
                );
                return Ok(PreparedScriptExecutionOutcome::DeferredModuleCompletion);
            }
        }
        let uses_runtime_owned_page_task_execution =
            self.prepared_script_uses_runtime_owned_page_task_execution(script);
        // DynamicScriptOwner produces the observable terminal and exact
        // lifecycle settlement after this execution phase.
        let skip_current_script_load_enqueue = dynamic_script_owner_id.is_some();
        let finish_behavior = if uses_runtime_owned_page_task_execution {
            PreparedScriptFinishBehavior::QueueRuntimeContinuation
        } else {
            PreparedScriptFinishBehavior::FlushPendingWork
        };
        self.finish_run_prepared_script(
            loader,
            script,
            false,
            skip_current_script_load_enqueue,
            finish_behavior,
        )
        .await
        .map_err(|message| {
            PreparedScriptExecutionError::from_message(message).with_body_activity(body_activity)
        })?;
        Ok(PreparedScriptExecutionOutcome::Completed(body_activity))
    }

    fn completion_owner_for_prepared_module_script(
        &self,
        script: &PreparedScript,
    ) -> ModuleScriptCompletionOwner {
        if self.prepared_script_uses_runtime_owned_page_task_execution(script) {
            ModuleScriptCompletionOwner::Runtime
        } else {
            ModuleScriptCompletionOwner::Parser
        }
    }

    fn module_script_continuation_for_prepared_script(
        &self,
        script: &PreparedScript,
        dynamic_script_owner_id: Option<crate::dynamic_script_owner::DynamicScriptOwnerId>,
        document_owner_before_run: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> std::result::Result<ModuleScriptContinuation, PreparedScriptExecutionError> {
        match self.completion_owner_for_prepared_module_script(script) {
            ModuleScriptCompletionOwner::Parser => {
                let pending_script_id = self
                    .document_runtime
                    .parser_module_document_scripts()
                    .pending_script_id_for_script(script)
                    .ok_or_else(|| {
                    PreparedScriptExecutionError::from_message(format!(
                        "parser-owned module script `{}` has no unique registered PendingScript",
                        script.url
                    ))
                })?;
                debug_assert_eq!(
                    pending_script_id.owner().task_owner(),
                    document_owner_before_run,
                    "parser module continuation must retain its admitted Document owner"
                );
                Ok(ModuleScriptContinuation::new_parser(
                    script.clone(),
                    pending_script_id,
                ))
            }
            ModuleScriptCompletionOwner::Runtime => {
                let owner = dynamic_script_owner_id.ok_or_else(|| {
                    PreparedScriptExecutionError::from_message(format!(
                        "runtime-owned module script `{}` has no dynamic script owner",
                        script.url
                    ))
                })?;
                Ok(ModuleScriptContinuation::new_runtime(
                    script.clone(),
                    owner,
                    document_owner_before_run,
                ))
            }
        }
    }

    pub(super) async fn settle_prepared_module_success(
        &mut self,
        loader: &ResourceRequestClient,
        script: &PreparedScript,
        document_owner_before_run: crate::frame_owner_model::FrameDocumentTaskOwner,
        dynamic_script_owner_id: Option<crate::dynamic_script_owner::DynamicScriptOwnerId>,
        evaluation: ParserModuleEvaluationSettlement,
        terminal_disposition: ParserModuleTerminalDisposition,
        prepared_activity: PreparedScriptBodyActivity,
    ) -> std::result::Result<PreparedModuleSuccessSettlement, String> {
        if self.script_run_replaced_document(document_owner_before_run, script) {
            return Ok(PreparedModuleSuccessSettlement::Stale);
        }
        let uses_runtime_owned_page_task_execution =
            self.prepared_script_uses_runtime_owned_page_task_execution(script);
        if !uses_runtime_owned_page_task_execution
            && terminal_disposition == ParserModuleTerminalDisposition::ReturnToSelectedParserTask
        {
            let script_event = self
                .plan_script_load_lifecycle_work_for_prepared_script(script)
                .and_then(|work| match work {
                    PostParseLifecycleWork::DispatchScriptEvent(task) => Some(task),
                    other => {
                        debug_assert!(
                            false,
                            "parser module load planning produced non-event work: {other:?}"
                        );
                        None
                    }
                });
            return Ok(PreparedModuleSuccessSettlement::ParserOwned(
                ParserOwnedModuleSuccessTerminal::new(evaluation, script_event, prepared_activity),
            ));
        }
        // DynamicScriptOwner produces the observable terminal and exact
        // lifecycle settlement after module evaluation starts.
        let skip_current_script_load_enqueue = dynamic_script_owner_id.is_some();
        let finish_behavior = if uses_runtime_owned_page_task_execution {
            PreparedScriptFinishBehavior::QueueRuntimeContinuation
        } else {
            PreparedScriptFinishBehavior::FlushPendingWork
        };
        self.finish_run_prepared_script(
            loader,
            script,
            false,
            skip_current_script_load_enqueue,
            finish_behavior,
        )
        .await?;
        Ok(if uses_runtime_owned_page_task_execution {
            PreparedModuleSuccessSettlement::RuntimeOwned
        } else {
            PreparedModuleSuccessSettlement::ParserOwnedCompleted
        })
    }

    async fn finish_run_prepared_script(
        &mut self,
        loader: &ResourceRequestClient,
        script: &PreparedScript,
        defer_script_event_dispatches: bool,
        skip_current_script_load_enqueue: bool,
        finish_behavior: PreparedScriptFinishBehavior,
    ) -> std::result::Result<(), String> {
        if self.has_pending_location_navigation() {
            debug!(url = %script.url, "run_prepared_script exiting due to pending location navigation");
            return Ok(());
        }
        if defer_script_event_dispatches {
            self.document_runtime
                .deferred_page_tasks_mut()
                .enter_scope();
        }
        self.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| -> std::result::Result<(), String> {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, &self.page_default_context);
                let scope = &mut v8::ContextScope::new(scope, context);
                let checkpoint_started = moli_trace::cdp_nav_timing_enabled().then(Instant::now);
                Self::reset_dom_binding_trace_window();
                let dom_binding_checkpoint_started =
                    moli_trace::dom_binding_timing_enabled().then(Instant::now);
                Self::perform_microtask_checkpoints(scope, Some(&script.url))
                    .map_err(|error| error.to_string())?;
                if let Some(started) = dom_binding_checkpoint_started {
                    Self::emit_dom_binding_trace_window(
                        "renderer_prepared_script_dom_binding_summary",
                        "post_script_checkpoint",
                        Some(&script.url),
                        started.elapsed(),
                    );
                }
                if let Some(started) = checkpoint_started {
                    tracing::info!(
                        target: "moli_cdp_nav_timing",
                        url = %script.url,
                        stage = "renderer_prepared_script_checkpoint_done",
                        elapsed_ms = started.elapsed().as_millis(),
                    );
                }
                Ok(())
            })?;
        let current_script_load_disposition = if skip_current_script_load_enqueue {
            FollowupPageTaskDisposition::Skipped
        } else {
            self.enqueue_script_load_lifecycle_work_for_prepared_script_best_effort(script)
        };
        if defer_script_event_dispatches {
            self.document_runtime.deferred_page_tasks_mut().exit_scope();
        }
        if defer_script_event_dispatches && skip_current_script_load_enqueue {
            debug!(
                url = %script.url,
                "run_prepared_script defers followup runtime work until parser progress after handling current script load"
            );
            return Ok(());
        }
        if self.pause_runtime_script_work_at_followup_task_boundary(current_script_load_disposition)
        {
            debug!(
                url = %script.url,
                disposition = ?current_script_load_disposition,
                "run_prepared_script finished after enqueueing current script load page task"
            );
            return Ok(());
        }
        let result = match finish_behavior {
            PreparedScriptFinishBehavior::FlushPendingWork => {
                self.flush_pending_work(loader, !defer_script_event_dispatches)
                    .await
            }
            PreparedScriptFinishBehavior::QueueRuntimeContinuation => {
                self.enqueue_immediate_runtime_script_work_if_needed();
                Ok(())
            }
        };
        debug!(
            url = %script.url,
            result = ?result.as_ref().map(|_| ()).map_err(|error| error.as_str()),
            "run_prepared_script finished"
        );
        result
    }

    fn parser_owned_external_classic_has_completion_event(&self, script: &PreparedScript) -> bool {
        script.kind == ScriptKind::Classic
            && script.source_kind == ScriptSourceKind::External
            && matches!(script.mode, ScriptMode::Normal | ScriptMode::Defer)
            && script.host_script_handle.as_deref().is_some_and(|handle| {
                self.document_runtime.script_handle_source(handle)
                    == ScriptHandleSource::ParserOwned
            })
    }

    fn plan_parser_owned_external_classic_completion_event(
        &mut self,
        script: &PreparedScript,
        kind: ScriptEventKind,
    ) -> Option<ScriptEventTask> {
        if !self.parser_owned_external_classic_has_completion_event(script) {
            return None;
        }
        let followup = match kind {
            ScriptEventKind::Load => "parser-owned classic script load",
            ScriptEventKind::Error => "parser-owned classic script error",
        };
        let handle =
            self.required_host_script_handle_for_observable_script_followup(script, followup)?;
        self.document_runtime
            .plan_script_event_task_for_script(kind, script, handle)
    }

    fn prepared_script_followup_lane(&self, script: &PreparedScript) -> DeferredPageTaskLane {
        if let Some(handle) = script.host_script_handle.as_deref()
            && let Some(lane) = self.document_runtime.script_handle_followup_lane(handle)
        {
            return lane;
        }
        crate::host::HostScriptScheduler::followup_lane_for_script(
            ScriptHandleSource::Unknown,
            script.mode,
        )
    }

    pub(super) fn parser_owned_inline_importmap_reports_window_error_immediately(
        &self,
        script: &PreparedScript,
    ) -> bool {
        script.kind == ScriptKind::ImportMap
            && script.source_kind == ScriptSourceKind::Inline
            && script.host_script_handle.as_deref().is_some_and(|handle| {
                self.document_runtime.script_handle_source(handle)
                    == ScriptHandleSource::ParserOwned
            })
    }

    pub(super) fn parser_owned_module_reports_failure_immediately(
        &self,
        script: &PreparedScript,
    ) -> bool {
        script.kind == ScriptKind::Module
            && script.host_script_handle.as_deref().is_some_and(|handle| {
                self.document_runtime.script_handle_source(handle)
                    == ScriptHandleSource::ParserOwned
            })
    }

    fn script_run_replaced_document(
        &mut self,
        document_owner_before_run: crate::frame_owner_model::FrameDocumentTaskOwner,
        script: &PreparedScript,
    ) -> bool {
        if self.current_main_document_task_owner() == Some(document_owner_before_run) {
            return false;
        }
        self.refresh_script_vm_local_document_state();
        debug!(
            url = %script.url,
            mode = ?script.mode,
            kind = ?script.kind,
            "skipping stale script followup after document replacement during script execution"
        );
        true
    }

    fn required_host_script_handle_for_observable_script_followup<'a>(
        &mut self,
        script: &'a PreparedScript,
        followup: &'static str,
    ) -> Option<&'a str> {
        let handle = script.host_script_handle.as_deref();
        if handle.is_none() {
            let node_is_live_script = self
                .document_runtime
                .dom_host()
                .node(script.node_id)
                .is_some_and(|node| node.is_connected() && node.is_script_element());
            self.record_runtime_warning(format_args!(
                "skipping {followup} for `{}` because prepared script has no host handle",
                script.url
            ));
            debug_assert!(
                !node_is_live_script,
                "observable script followup requires a bound host handle"
            );
        }
        handle
    }

    fn prepared_script_is_live_for_execution(&mut self, script: &PreparedScript) -> bool {
        let Some(handle) = script.host_script_handle.as_deref() else {
            let allow_missing_handle = script.kind == ScriptKind::Classic
                && script.source_kind == ScriptSourceKind::Inline
                && script.mode == ScriptMode::Normal;
            let node_is_live_script = self
                .document_runtime
                .dom_host()
                .node(script.node_id)
                .is_some_and(|node| node.is_connected() && node.is_script_element());
            if !allow_missing_handle {
                self.record_runtime_warning(format_args!(
                    "skipping stale prepared script `{}` because it has no bound host handle",
                    script.url
                ));
                debug_assert!(
                    !node_is_live_script,
                    "prepared script execution requires a bound host handle"
                );
            }
            return allow_missing_handle;
        };

        if self
            .document_runtime
            .resolve_host_script_handle(handle)
            .is_some()
        {
            return true;
        }

        self.record_runtime_warning(format_args!(
            "skipping stale prepared script `{}` because handle `{handle}` is no longer live",
            script.url
        ));
        debug_assert!(
            false,
            "prepared script execution requires a live registered handle"
        );
        false
    }

    /// Run one explicit page-task microtask checkpoint before a queued script task.
    ///
    /// This does not make the runtime a full browser task queue yet. The point is
    /// narrower: once parse-time classic async is modeled as a page-owned task, the
    /// task should get its own pre-task checkpoint instead of being executed as
    /// "whatever ready scripts coordinator had in a vector right now".
    ///
    /// Keeping this helper separate from `run_prepared_script(...)` makes the
    /// boundary explicit:
    /// - pre-task checkpoint belongs to the page task queue turn
    /// - post-script checkpoint still belongs to script execution / flush logic
    pub(super) fn perform_script_task_checkpoint(
        &mut self,
        script_url: Option<&Url>,
    ) -> anyhow::Result<()> {
        self.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, &self.page_default_context);
                let scope = &mut v8::ContextScope::new(scope, context);
                Self::reset_dom_binding_trace_window();
                let dom_binding_checkpoint_started =
                    moli_trace::dom_binding_timing_enabled().then(Instant::now);
                Self::perform_microtask_checkpoints(scope, script_url)?;
                if let Some(started) = dom_binding_checkpoint_started {
                    Self::emit_dom_binding_trace_window(
                        "renderer_script_task_dom_binding_summary",
                        "pre_task_checkpoint",
                        script_url,
                        started.elapsed(),
                    );
                }
                Ok(())
            })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn snapshot_globals(&mut self) -> Result<Option<BTreeMap<String, JsValueSnapshot>>> {
        let baseline_json = serde_json::to_string(&self.baseline_globals)
            .context("failed to encode baseline globals for script snapshot")?;
        let snapshot_source = format!(
            r#"
(() => {{
  const baseline = Object.create(null);
  for (const name of {baseline_json}) {{
    baseline[name] = true;
  }}
  const out = {{}};
  const describeFiniteNumberFallback = (value) => {{
    if (Number.isNaN(value)) {{
      return "NaN";
    }}
    if (value === Infinity) {{
      return "Infinity";
    }}
    if (value === -Infinity) {{
      return "-Infinity";
    }}
    return "number";
  }};
  const describeUnsupported = (value) => {{
    switch (typeof value) {{
      case "bigint":
        return "bigint";
      case "function":
        return "[function]";
      case "object": {{
        try {{
          return Array.isArray(value) ? "[array]" : "[object]";
        }} catch (_error) {{
          // Array.isArray throws for a revoked Proxy. Snapshotting is
          // best-effort observation and must not fail the whole page merely
          // because an unsupported object can no longer be inspected.
          return "[object]";
        }}
      }}
      case "symbol":
        return "symbol";
      default:
        return "<unsupported>";
    }}
  }};
  const isArrayIndexName = (name) => {{
    const index = name >>> 0;
    return index !== 4294967295 && String(index) === name;
  }};
  for (const name of Object.getOwnPropertyNames(globalThis)) {{
    if (baseline[name] === true || isArrayIndexName(name)) {{
      continue;
    }}

    const descriptor = Object.getOwnPropertyDescriptor(globalThis, name);
    if (!descriptor || !("value" in descriptor)) {{
      out[name] = {{ kind: "unsupported", value: "[accessor]" }};
      continue;
    }}

    const value = descriptor.value;
    if (value === undefined) {{
      out[name] = {{ kind: "undefined" }};
      continue;
    }}

    if (value === null) {{
      out[name] = {{ kind: "null" }};
      continue;
    }}

    switch (typeof value) {{
      case "boolean":
        out[name] = {{ kind: "boolean", value }};
        break;
      case "number":
        out[name] = Number.isFinite(value)
          ? {{ kind: "number", value }}
          : {{ kind: "unsupported", value: describeFiniteNumberFallback(value) }};
        break;
      case "string":
        out[name] = {{ kind: "string", value }};
        break;
      default:
        out[name] = {{ kind: "unsupported", value: describeUnsupported(value) }};
        break;
    }}
  }}
  return JSON.stringify(out);
}})()
"#
        );
        let default_context = &self.page_default_context as *const _;
        let snapshot_json = self
            .eval_string_in_context_ptr_internal_snapshot(default_context, &snapshot_source)
            .context("failed to evaluate script state snapshot")?;
        let snapshot = serde_json::from_str::<BTreeMap<String, SerializedJsValue>>(&snapshot_json)
            .context("failed to deserialize script state snapshot")?;

        Ok(Some(
            snapshot
                .into_iter()
                .map(|(name, value)| (name, value.into_snapshot()))
                .collect(),
        ))
    }

    #[cfg(not(any(test, feature = "test-support")))]
    pub(super) fn snapshot_globals(&mut self) -> Result<Option<BTreeMap<String, JsValueSnapshot>>> {
        let _ = &self.baseline_globals;
        Ok(None)
    }

    pub(super) fn take_runtime_observable_report_output(
        &mut self,
    ) -> Result<ScriptObservableOutput> {
        self.sync_runtime_observable_source_events()?;
        Ok(self
            .runtime_observable_source_queue
            .take_report_observable_output(
                self.runtime_observable_default_execution_context_id(),
                self.page_default_runtime_observable_context_token,
            ))
    }

    pub(super) fn snapshot_console_messages_with_context(
        &mut self,
    ) -> Result<Vec<RuntimeConsoleMessageSnapshot>> {
        let contexts = self.page_runtime_observable_contexts();
        let mut messages = Vec::new();
        for context in contexts {
            let Some(execution_context_id) = context.execution_context_id else {
                continue;
            };
            let mut context_messages =
                self.snapshot_console_message_details_in_context(context.context)?;
            for mut message in context_messages.drain(..) {
                if let Some(object) = message.as_object_mut() {
                    object.insert("executionContextId".to_owned(), json!(execution_context_id));
                }
                messages.push(
                    serde_json::from_value::<RuntimeConsoleMessageSnapshot>(message)
                        .context("runtime console message snapshot has invalid shape")?,
                );
            }
        }
        Ok(messages)
    }

    fn page_runtime_observable_contexts(&self) -> Vec<PageRuntimeObservableContext> {
        let mut contexts = vec![PageRuntimeObservableContext {
            execution_context_id: self.runtime_observable_default_execution_context_id(),
            context_token: self.page_default_runtime_observable_context_token,
            context: &self.page_default_context as *const _,
        }];
        let mut isolated_context_ids = self
            .page_isolated_world_contexts
            .execution_context_ids()
            .collect::<Vec<_>>();
        isolated_context_ids.sort_unstable();
        for execution_context_id in isolated_context_ids {
            if let Some(world) = self
                .page_isolated_world_contexts
                .context(execution_context_id)
            {
                contexts.push(PageRuntimeObservableContext {
                    execution_context_id: Some(execution_context_id),
                    context_token: world.runtime_observable_context_token,
                    context: &world.context as *const _,
                });
            }
        }
        let mut child_default_context_ids = self
            .child_frame_realm_store
            .execution_context_ids()
            .collect::<Vec<_>>();
        child_default_context_ids.sort_unstable();
        for execution_context_id in child_default_context_ids {
            if let Some(world) = self.child_frame_realm_store.get(&execution_context_id) {
                contexts.push(PageRuntimeObservableContext {
                    execution_context_id: Some(execution_context_id),
                    context_token: world.runtime_observable_context_token,
                    context: &world.context as *const _,
                });
            }
        }
        contexts
    }

    fn sync_runtime_observable_source_events(&mut self) -> Result<()> {
        let contexts = self.page_runtime_observable_contexts();
        let active_tokens = contexts
            .iter()
            .map(|context| context.context_token)
            .collect::<BTreeSet<_>>();
        let token_to_execution_context_id = contexts
            .iter()
            .filter_map(|context| {
                context
                    .execution_context_id
                    .map(|execution_context_id| (context.context_token, execution_context_id))
            })
            .collect::<BTreeMap<_, _>>();
        let active_contexts = contexts
            .iter()
            .filter_map(|context| context.execution_context_id)
            .collect::<BTreeSet<_>>();
        let mut host = self._context_host.borrow_mut();
        let pending_console_events = host.take_pending_runtime_observable_console_source_events();
        drop(host);
        self.runtime_observable_source_queue.sync_source_events(
            &active_contexts,
            &active_tokens,
            &token_to_execution_context_id,
            pending_console_events,
        );
        Ok(())
    }

    pub(super) fn settle_renderer_output_publication(
        &mut self,
    ) -> Option<crate::runtime::RendererOutputPublication> {
        self.page_inspector
            .devtools_target()
            .pause_ref()
            .finish_owner_turn();
        self.sync_runtime_observable_source_events()
            .expect("runtime observable source synchronization should be infallible");
        let environment = self.renderer_page_script_environment.as_ref()?;
        let output_journal = environment.output_journal();
        let mut pending = output_journal.take_pending_for_resolution()?;

        let current_agent_token = self.page_inspector.agent_token();
        for record in pending.records_mut() {
            record.with_runtime_inspector_batch_mut(|batch| {
                let raw_messages = batch
                    .messages
                    .iter()
                    .cloned()
                    .map(RendererRuntimeInspectorMessage::into_v8_inspector_message)
                    .collect::<Vec<_>>();
                self.page_inspector
                    .record_execution_context_state(&raw_messages, self.root_frame_id.as_deref());
                self.page_isolated_world_contexts
                    .record_inspector_context_state(&raw_messages, self.root_frame_id.as_deref());
                if batch.agent_token == current_agent_token {
                    batch.v8_state_update =
                        self.inspector_v8_session_state(batch.session.wire_session_id());
                }
            });
        }
        Some(pending.finish())
    }

    pub(super) fn append_renderer_output_records(
        &self,
        records: impl IntoIterator<Item = crate::runtime::PendingRendererOutputRecord>,
    ) {
        let environment = self
            .renderer_page_script_environment
            .as_ref()
            .expect("a live Page command must have a renderer output journal");
        environment.output_journal().append_records(records);
    }

    pub(super) fn append_renderer_command_output_records(
        &self,
        records: Vec<crate::runtime::PendingRendererOutputRecord>,
    ) -> crate::runtime::RendererOutputCursor {
        let environment = self
            .renderer_page_script_environment
            .as_ref()
            .expect("a live Page command must have a renderer output journal");
        environment.output_journal().append_command_records(records)
    }

    pub(super) fn has_renderer_output_journal(&self) -> bool {
        self.renderer_page_script_environment.is_some()
    }

    #[cfg(test)]
    pub(crate) fn bind_renderer_output_journal_for_test(
        &mut self,
        output_journal: crate::runtime::RendererTurnOutputJournal,
    ) {
        self._context_host
            .borrow_mut()
            .bind_output_journal(output_journal);
    }

    pub(super) fn renderer_output_tail_cursor(
        &self,
    ) -> Option<crate::runtime::RendererOutputCursor> {
        self.renderer_page_script_environment
            .as_ref()
            .and_then(|environment| environment.output_journal().last_published_cursor())
    }

    pub(super) fn declare_renderer_output_fence(
        &self,
        cursor: crate::runtime::RendererOutputCursor,
    ) -> crate::runtime::RendererOutputFence {
        self.renderer_page_script_environment
            .as_ref()
            .expect("a renderer output fence requires a Page script environment")
            .output_journal()
            .declare_fence(cursor)
    }

    pub(super) fn renderer_document_isolate_ops(
        &mut self,
    ) -> ScriptVmRendererDocumentIsolateOps<'_> {
        ScriptVmRendererDocumentIsolateOps { vm: self }
    }

    fn renderer_document_isolate_heap_usage(&mut self) -> Result<RendererRuntimeHeapUsage> {
        let moli_counters = self.moli_memory_counters();
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let stats = isolate.get_heap_statistics();
                let heap_spaces = (0..isolate.number_of_heap_spaces())
                    .filter_map(|index| {
                        isolate.get_heap_space_statistics(index).map(|space| {
                            RendererRuntimeHeapSpaceUsage {
                                name: space.space_name().to_string_lossy().into_owned(),
                                size: space.space_size(),
                                used_size: space.space_used_size(),
                                available_size: space.space_available_size(),
                                physical_size: space.physical_space_size(),
                            }
                        })
                    })
                    .collect::<Vec<_>>();
                Ok(RendererRuntimeHeapUsage {
                    used_size: stats.used_heap_size(),
                    total_size: stats.total_heap_size(),
                    total_heap_size_executable: stats.total_heap_size_executable(),
                    total_physical_size: stats.total_physical_size(),
                    total_available_size: stats.total_available_size(),
                    heap_size_limit: stats.heap_size_limit(),
                    malloced_memory: stats.malloced_memory(),
                    peak_malloced_memory: stats.peak_malloced_memory(),
                    external_memory: stats.external_memory(),
                    number_of_native_contexts: stats.number_of_native_contexts(),
                    number_of_detached_contexts: stats.number_of_detached_contexts(),
                    total_allocated_bytes: stats.total_allocated_bytes(),
                    total_global_handles_size: stats.total_global_handles_size(),
                    used_global_handles_size: stats.used_global_handles_size(),
                    heap_spaces,
                    moli: moli_counters,
                })
            })
    }

    fn moli_memory_counters(&self) -> RendererMoliMemoryDiagnostics {
        let document = self.document_runtime.snapshot_document();
        let dom = moli_dom_memory_counters(&document);
        let main_window_proxy_identity_hash = self
            .renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, &self.page_default_context);
                Ok(context.global(scope).get_identity_hash().get())
            })
            .ok();
        let host = self._context_host.borrow();
        RendererMoliMemoryDiagnostics {
            scope: RendererMoliMemoryScopeDiagnostics {
                v8_heap: "page-vm-document-isolate",
                v8_heap_is_target_local: true,
                counters: "target-document",
                garbage_collection: "page-vm-document-isolate",
            },
            dom,
            runtime: RendererMoliRuntimeMemoryDiagnostics {
                runtime_observable_context_count: self.page_runtime_observable_contexts().len(),
                isolated_context_count: self.page_isolated_world_contexts.len(),
                child_default_context_count: self.child_frame_realm_store.len(),
                child_browsing_context_count: host.child_browsing_context_count(),
                pending_subresource_requests: host.pending_subresource_request_count(),
                pending_subresource_fetch_infos: host.pending_subresource_fetch_info_count(),
                pending_subresource_continue_events: host
                    .pending_subresource_continue_event_count(),
                pending_runtime_binding_calls: self
                    .document_runtime
                    .pending_runtime_binding_call_count(),
                completed_child_frame_navigation_loads: host
                    .completed_child_frame_navigation_load_count(),
                pending_inspector_messages: self.page_inspector.outbound_len(),
                inspector_session_registry_owner: self
                    .page_inspector
                    .registry_owner_for_diagnostics(),
                inspector_session_registry_lifetime_scope: self
                    .page_inspector
                    .registry_lifetime_scope_for_diagnostics(),
                inspector_session_count: self.page_inspector.session_count_for_diagnostics(),
                inspector_context_group_id: self.page_inspector.context_group_id_for_diagnostics(),
                inspector_context_group_scope: self
                    .page_inspector
                    .registry_lifetime_scope_for_diagnostics(),
                inspector_context_registration_count: self
                    .page_inspector
                    .context_registration_count_for_diagnostics(),
                main_window_proxy_identity_hash,
                inspector_default_context_registry_count: self
                    .renderer_document_isolate
                    .renderer_document_isolate_inspector_default_context_registry_count(),
                inspector_default_context_registry_scope: "page-vm-document-isolate",
                v8_foreground_task_wake_scope: "page-vm-document-isolate",
                v8_foreground_task_wake_context_group_id_available: false,
                v8_foreground_task_wake_internal_policy: "typed-page-source-and-owner-scheduler",
                v8_foreground_task_wake_external_policy: "post-turn-runtime-output",
            },
            script_execution: self.script_execution_memory.to_diagnostics(),
        }
    }

    fn collect_renderer_document_isolate_garbage(&mut self) -> Result<()> {
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                isolate.memory_pressure_notification(v8::MemoryPressureLevel::Critical);
                isolate.low_memory_notification();
                Ok(())
            })
    }

    fn notify_renderer_document_isolate_memory_pressure(&mut self) -> Result<()> {
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let stats = isolate.get_heap_statistics();
                if renderer_document_isolate_critical_pressure_required(
                    stats.used_heap_size(),
                    stats.heap_size_limit(),
                ) {
                    // One renderer process can host several Page isolates. A
                    // target with a rapidly replaced child realm must not hit
                    // V8's process-fatal heap limit before its owner can close
                    // that target. Escalate the existing periodic maintenance
                    // once this isolate consumes a third of its own limit.
                    isolate.memory_pressure_notification(v8::MemoryPressureLevel::Critical);
                    isolate.low_memory_notification();
                } else {
                    isolate.memory_pressure_notification(v8::MemoryPressureLevel::Moderate);
                }
                Ok(())
            })
    }

    fn snapshot_console_messages_in_context(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
    ) -> Result<Vec<String>> {
        // SAFETY: callers pass pointers to `self.page_default_context` or page realm context entries owned by this `ScriptVm`.
        // The snapshot operation only reads a context slot; it does not mutate or remove any
        // context while the raw pointer is used.
        // This is an internal console snapshot, not an owner-visible script turn.
        self.with_context_scope_by_ptr(context_ptr, |scope, _| {
            Ok(crate::context_bootstrap::snapshot_console_messages_for_current_context(scope))
        })
        .context("failed to snapshot console output")
    }

    fn snapshot_console_message_details_in_context(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
    ) -> Result<Vec<Value>> {
        // SAFETY: callers pass pointers to `self.page_default_context` or page realm context entries owned by this `ScriptVm`.
        // The snapshot operation only reads a context slot; it does not mutate or remove any
        // context while the raw pointer is used.
        // This is an internal console snapshot, not an owner-visible script turn.
        let mut details = self
            .with_context_scope_by_ptr(context_ptr, |scope, _| {
                Ok(
                    crate::context_bootstrap::snapshot_console_message_details_for_current_context(
                        scope,
                    ),
                )
            })
            .context("failed to snapshot console detail output")?;
        if !details.is_empty() {
            return Ok(details);
        }

        details = self
            .snapshot_console_messages_in_context(context_ptr)?
            .into_iter()
            .map(|message| {
                let text = message
                    .split_once(": ")
                    .map(|(_, text)| text)
                    .unwrap_or(message.as_str())
                    .to_owned();
                json!({
                    "message": message,
                    "text": text,
                    "args": [
                        {
                            "type": "string",
                            "value": text,
                        }
                    ],
                })
            })
            .collect();
        Ok(details)
    }
}

impl ScriptVmRendererDocumentIsolateOps<'_> {
    pub(super) fn renderer_document_isolate_heap_usage(
        &mut self,
    ) -> Result<RendererRuntimeHeapUsage> {
        self.vm.renderer_document_isolate_heap_usage()
    }

    pub(super) fn collect_renderer_document_isolate_garbage(&mut self) -> Result<()> {
        self.vm.collect_renderer_document_isolate_garbage()
    }

    pub(super) fn notify_renderer_document_isolate_memory_pressure(&mut self) -> Result<()> {
        self.vm.notify_renderer_document_isolate_memory_pressure()
    }
}

impl ScriptVm {
    pub(super) fn take_network_output(&mut self) -> super::types::ScriptNetworkOutput {
        self._context_host.borrow_mut().take_network_output()
    }

    pub(super) fn subresource_activity_epoch(&self) -> u64 {
        self._context_host.borrow().subresource_activity_epoch()
    }

    #[cfg(test)]
    pub(super) fn take_pending_subresource_fetch_infos(
        &mut self,
    ) -> Vec<PendingSubresourceFetchInfo> {
        self._context_host
            .borrow_mut()
            .take_pending_subresource_fetch_infos()
    }

    #[cfg(test)]
    pub(super) fn take_pending_subresource_continue_events(
        &mut self,
    ) -> Vec<PendingSubresourceContinueEvent> {
        self._context_host
            .borrow_mut()
            .take_pending_subresource_continue_events()
    }

    #[cfg(test)]
    pub(super) fn take_pending_file_chooser_activations(
        &mut self,
    ) -> Vec<crate::RendererPendingFileChooserActivation> {
        self._context_host
            .borrow_mut()
            .take_pending_file_chooser_activations()
    }

    #[cfg(test)]
    pub(super) fn take_pending_download_activations(
        &mut self,
    ) -> Vec<crate::RendererPendingDownloadActivation> {
        self._context_host
            .borrow_mut()
            .take_pending_download_activations()
    }

    #[cfg(test)]
    pub(super) fn take_pending_javascript_dialogs(
        &mut self,
    ) -> Vec<crate::RendererPendingJavaScriptDialog> {
        self._context_host
            .borrow_mut()
            .take_pending_javascript_dialogs()
    }

    pub(super) fn set_javascript_dialog_handler_enabled(&mut self, enabled: bool) {
        self._context_host
            .borrow_mut()
            .set_javascript_dialog_handler_enabled(enabled);
    }

    #[cfg(test)]
    pub(super) fn take_pending_popup_activations(
        &mut self,
    ) -> Vec<crate::RendererPendingPopupActivation> {
        self._context_host
            .borrow_mut()
            .take_pending_popup_activations()
    }

    #[cfg(test)]
    pub(super) fn take_completed_child_frame_navigation_loads(
        &mut self,
    ) -> Vec<crate::protocol_types::ChildFrameNavigationSnapshot> {
        self._context_host
            .borrow_mut()
            .take_completed_child_frame_navigation_loads()
            .into_iter()
            .map(
                |snapshot| crate::protocol_types::ChildFrameNavigationSnapshot {
                    frame_id: snapshot.frame_id,
                    parent_frame_id: snapshot.parent_frame_id,
                    loader_id: snapshot.loader_id,
                    name: snapshot.name,
                    url: snapshot.url,
                    document_open_replacement: snapshot.document_open_replacement,
                    security_origin_inherited: snapshot.security_origin_inherited,
                    security_origin_opaque: snapshot.security_origin_opaque,
                    document_network: snapshot.document_network,
                },
            )
            .collect()
    }

    #[cfg(test)]
    pub(super) fn take_completed_child_document_networks(
        &mut self,
    ) -> Vec<crate::protocol_types::ChildFrameDocumentNetworkActivitySnapshot> {
        self._context_host
            .borrow_mut()
            .take_completed_child_document_networks()
    }

    #[cfg(test)]
    pub(super) fn take_pending_child_frame_tree_events(
        &mut self,
    ) -> Vec<crate::protocol_types::ChildFrameTreeEventSnapshot> {
        self._context_host
            .borrow_mut()
            .take_pending_child_frame_tree_events()
    }

    #[cfg(test)]
    pub(super) fn completed_child_frame_navigation_load_count(&self) -> usize {
        self._context_host
            .borrow()
            .completed_child_frame_navigation_load_count()
    }

    #[cfg(test)]
    pub(super) fn take_runtime_binding_calls(&mut self) -> Vec<PendingRuntimeBindingCall> {
        drain_internal_runtime_binding_calls(self);
        self.document_runtime.take_runtime_binding_calls()
    }

    pub(super) fn take_runtime_inspector_messages(
        &mut self,
        inspector_session_id: Option<&str>,
    ) -> Vec<RendererRuntimeInspectorMessage> {
        let messages = self
            .page_inspector
            .take_outbound_messages_for_session(inspector_session_id);
        self.page_inspector
            .record_execution_context_state(&messages, self.root_frame_id.as_deref());
        self.page_isolated_world_contexts
            .record_inspector_context_state(&messages, self.root_frame_id.as_deref());
        self.runtime_inspector_messages_from_v8_messages(messages)
    }

    pub(super) fn devtools_agent_token(&self) -> moli_page_types::RendererDevToolsAgentToken {
        self.page_inspector.agent_token()
    }

    pub(super) fn inspector_v8_session_state(
        &self,
        inspector_session_id: Option<&str>,
    ) -> Option<moli_page_types::V8InspectorSessionState> {
        self.renderer_document_isolate
            .with_renderer_document_isolate_and_inspector_mut(|_, _| {
                self.page_inspector.v8_session_state(inspector_session_id)
            })
    }

    pub(super) fn inspector_v8_session_states(
        &self,
    ) -> Vec<(
        moli_page_types::DevToolsSessionKey,
        moli_page_types::V8InspectorSessionState,
    )> {
        self.renderer_document_isolate
            .with_renderer_document_isolate_and_inspector_mut(|_, _| {
                self.page_inspector.v8_session_states()
            })
    }

    pub(super) fn reattach_v8_inspector_sessions(
        &self,
        restores: &[crate::runtime::RendererInspectorSessionRestoreSnapshot],
    ) {
        self.renderer_document_isolate
            .with_renderer_document_isolate_and_inspector_mut(|_, backend| {
                self.page_inspector.reattach_v8_sessions(backend, restores);
            });
    }

    pub(super) fn ensure_runtime_inspector_session(&mut self, inspector_session_id: Option<&str>) {
        let renderer_document_isolate = self.renderer_document_isolate.clone();
        let page_inspector = &self.page_inspector;
        renderer_document_isolate.with_renderer_document_isolate_and_inspector_mut(|_, backend| {
            page_inspector.ensure_frontend_session(backend, inspector_session_id);
        });
    }

    pub(super) fn page_diagnostics_snapshot(&mut self) -> Result<RendererPageDiagnosticsSnapshot> {
        self.sync_runtime_observable_source_events()?;
        let runtime_observable_source = self
            .runtime_observable_source_queue
            .snapshot(self.runtime_observable_default_execution_context_id());
        let runtime_console_messages_by_context = runtime_observable_source
            .as_ref()
            .map(|source| source.console_messages_by_context())
            .unwrap_or_default();
        let runtime_console_messages_with_context = runtime_observable_source
            .as_ref()
            .map(|source| source.console_messages_with_context())
            .unwrap_or_default();
        let runtime_lifecycle_errors = runtime_observable_source
            .as_ref()
            .map(|source| source.lifecycle_errors())
            .unwrap_or_default();
        let host = self._context_host.borrow();
        let mut snapshot =
            RendererPageDiagnosticsSnapshot::from_diagnostics(RendererActivityDiagnostics {
                document_context_count: 1
                    + self.page_isolated_world_contexts.len()
                    + self.child_frame_realm_store.len(),
                isolated_world_context_count: self.page_isolated_world_contexts.len(),
                child_default_context_count: self.child_frame_realm_store.len(),
                pending_subresource_requests: host.pending_subresource_request_count(),
                pending_subresource_fetch_infos: host.pending_subresource_fetch_info_count(),
                pending_subresource_continue_events: host
                    .pending_subresource_continue_event_count(),
                pending_file_chooser_activations: host.pending_file_chooser_activation_count(),
                pending_download_activations: host.pending_download_activation_count(),
                pending_popup_activations: host.pending_popup_activation_count(),
                pending_javascript_dialogs: host.pending_javascript_dialog_count(),
                pending_runtime_binding_calls: self
                    .document_runtime
                    .pending_runtime_binding_call_count(),
                pending_inspector_messages: self.page_inspector.outbound_len(),
                runtime_console_messages_with_context,
                runtime_console_messages_by_context,
                runtime_lifecycle_errors,
                completed_child_frame_navigation_loads: host
                    .completed_child_frame_navigation_load_count(),
                dedicated_worker_loading_count: host
                    .dedicated_worker_loading_count_for_diagnostics(),
                dedicated_worker_running_worker_isolate_count: host
                    .dedicated_worker_running_worker_isolate_count_for_diagnostics(),
                pending_webcrypto_tasks: host.pending_webcrypto_task_count(),
                pending_opfs_tasks: host.pending_opfs_task_count(),
            });
        snapshot.set_runtime_observable_source(runtime_observable_source);
        Ok(snapshot)
    }

    pub(super) fn dedicated_worker_running_worker_isolate_count_for_diagnostics(&self) -> usize {
        self._context_host
            .borrow()
            .dedicated_worker_running_worker_isolate_count_for_diagnostics()
    }

    pub(crate) fn has_pending_webcrypto_tasks(&self) -> bool {
        self._context_host.borrow().has_pending_webcrypto_tasks()
    }

    #[cfg(test)]
    pub(crate) fn pending_webcrypto_execution_contexts_for_test(
        &self,
    ) -> Vec<(
        crate::native_bridge::WindowExecutionContextOwner,
        crate::native_bridge::RuntimeObservableContextToken,
    )> {
        self._context_host
            .borrow()
            .pending_webcrypto_execution_contexts_for_test()
    }

    pub(crate) fn has_pending_opfs_tasks(&self) -> bool {
        self._context_host.borrow().has_pending_opfs_tasks()
    }

    pub(super) fn pending_subresource_request_count(&self) -> usize {
        self._context_host
            .borrow()
            .pending_subresource_request_count()
    }

    pub(crate) fn has_pending_native_module_job(&self) -> bool {
        self.has_pending_dynamic_module_job()
    }

    pub(crate) fn has_pending_dynamic_module_job(&self) -> bool {
        self.document_runtime
            .has_pending_native_dynamic_module_import()
            || self.has_pending_child_dynamic_module_import()
    }

    #[cfg(test)]
    pub(crate) fn has_ready_dynamic_module_job(&self) -> bool {
        self.document_runtime
            .has_ready_native_dynamic_module_import()
    }

    #[cfg(test)]
    pub(crate) fn has_inflight_dynamic_module_fetch(&self) -> bool {
        self.document_runtime
            .has_inflight_native_dynamic_module_import_fetch()
            || self.has_inflight_child_dynamic_module_import_fetch()
    }

    pub(crate) fn resource_scheduler(&self) -> RendererResourceScheduler {
        self._context_host.borrow().resource_scheduler()
    }

    pub(super) fn accept_parser_discovered_native_modulepreloads(
        &mut self,
        link_handles: impl IntoIterator<Item = crate::dom::native::NativeNodeId>,
    ) -> bool {
        let (preloads, runtime_warnings, link_error_tasks) = self
            .document_runtime
            .accept_parser_discovered_modulepreload_links(link_handles)
            .into_parts();
        let mut progressed = link_error_tasks > 0;
        for warning in runtime_warnings {
            self.record_runtime_warning(format_args!("{warning}"));
            progressed = true;
        }
        for preload in preloads {
            match self.register_native_modulepreload_for_owner(preload) {
                Ok(run) => progressed |= run.is_some(),
                Err(error) => self.record_runtime_warning(format_args!(
                    "parser-discovered modulepreload failed before fetch scheduling: {}",
                    error
                )),
            }
        }
        progressed
    }

    pub(super) fn evaluate_expression_payload_with_await(
        &mut self,
        expression: &str,
        await_promise: bool,
        user_gesture: bool,
    ) -> Result<Value> {
        let outcome = self.begin_runtime_evaluate(
            None,
            expression,
            await_promise,
            user_gesture,
            None,
            RuntimeEvaluateCodeGenerationPolicy::from_cdp(None),
            RuntimeEvaluateResultMode::RemoteObject,
        )?;
        self.require_completed_runtime_evaluate(outcome)
    }

    pub(super) fn evaluate_expression_payload_in_context_with_await(
        &mut self,
        execution_context_id: Option<i64>,
        expression: &str,
        await_promise: bool,
        user_gesture: bool,
        file_prompt_handler: Option<&str>,
    ) -> Result<Value> {
        let outcome = self.begin_runtime_evaluate(
            execution_context_id,
            expression,
            await_promise,
            user_gesture,
            file_prompt_handler,
            RuntimeEvaluateCodeGenerationPolicy::from_cdp(None),
            RuntimeEvaluateResultMode::RemoteObject,
        )?;
        self.require_completed_runtime_evaluate(outcome)
    }

    pub(super) fn evaluate_expression_by_value_payload_in_context_with_await(
        &mut self,
        execution_context_id: Option<i64>,
        expression: &str,
        await_promise: bool,
        user_gesture: bool,
        file_prompt_handler: Option<&str>,
    ) -> Result<Value> {
        let outcome = self.begin_runtime_evaluate(
            execution_context_id,
            expression,
            await_promise,
            user_gesture,
            file_prompt_handler,
            RuntimeEvaluateCodeGenerationPolicy::from_cdp(None),
            RuntimeEvaluateResultMode::ByValue,
        )?;
        self.require_completed_runtime_evaluate(outcome)
    }

    pub(crate) fn performance_metric_snapshot(
        &mut self,
    ) -> Result<RendererPerformanceMetricSnapshot> {
        let snapshot_json = self
            .eval(PERFORMANCE_METRICS_SNAPSHOT_EXPRESSION)
            .context("failed to evaluate performance metric snapshot")?;
        serde_json::from_str(&snapshot_json).context("failed to decode performance metric snapshot")
    }

    pub(crate) fn performance_metric_snapshot_without_script(
        &self,
        lifecycle: crate::runtime::RendererDocumentLifecycleSnapshot,
        resource_count: usize,
    ) -> RendererPerformanceMetricSnapshot {
        let started_micros = lifecycle.started.timestamp_micros;
        let timestamp_ms = |timestamp_micros: u64| timestamp_micros as f64 / 1_000.0;
        let dom = moli_dom_memory_counters(self.document_runtime.dom_host().dom());
        RendererPerformanceMetricSnapshot {
            time_origin_ms: Some(timestamp_ms(started_micros)),
            now_ms: Some(
                moli_time::monotonic_timestamp_micros().saturating_sub(started_micros) as f64
                    / 1_000.0,
            ),
            navigation_start_ms: Some(timestamp_ms(started_micros)),
            dom_content_loaded_ms: lifecycle
                .dom_content_loaded
                .map(|stamp| timestamp_ms(stamp.timestamp_micros)),
            load_event_ms: lifecycle
                .load
                .map(|stamp| timestamp_ms(stamp.timestamp_micros)),
            document_count: Some(1.0),
            frame_count: Some((1 + dom.iframe_element_count) as f64),
            node_count: Some(dom.node_count as f64),
            resource_count: Some(resource_count as f64),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn begin_runtime_evaluate(
        &mut self,
        execution_context_id: Option<i64>,
        expression: &str,
        await_promise: bool,
        user_gesture: bool,
        file_prompt_handler: Option<&str>,
        code_generation_policy: RuntimeEvaluateCodeGenerationPolicy,
        result_mode: RuntimeEvaluateResultMode,
    ) -> Result<RuntimeEvaluateOutcome> {
        self.validate_runtime_evaluate_context(execution_context_id)?;
        let call_id = self.next_internal_runtime_evaluate_call_id()?;
        let mut params = serde_json::Map::new();
        params.insert("expression".to_owned(), json!(expression));
        params.insert("awaitPromise".to_owned(), json!(await_promise));
        params.insert(
            "returnByValue".to_owned(),
            json!(result_mode.returns_by_value()),
        );
        params.insert(
            "allowUnsafeEvalBlockedByCSP".to_owned(),
            json!(code_generation_policy.allows_unsafe_eval_blocked_by_csp()),
        );
        if let Some(execution_context_id) = execution_context_id {
            params.insert("contextId".to_owned(), json!(execution_context_id));
        }
        if user_gesture {
            params.insert("userGesture".to_owned(), json!(true));
        }
        if let Some(file_prompt_handler) = file_prompt_handler {
            params.insert(
                WEBDRIVER_BIDI_FILE_PROMPT_HANDLER_PARAM.to_owned(),
                json!(file_prompt_handler),
            );
        }
        let raw_request = serde_json::to_string(&json!({
            "id": call_id,
            "method": "Runtime.evaluate",
            "params": params,
        }))
        .context("failed to encode internal Runtime.evaluate request")?;
        let (response_tx, mut response_rx) = tokio::sync::oneshot::channel();
        self.dispatch_internal_runtime_evaluate_protocol_message(
            &raw_request,
            RendererRuntimeInspectorResponseSender::new(call_id, response_tx),
        )?;
        match response_rx.try_recv() {
            Ok(completion) => self
                .runtime_evaluate_payload_from_completion(completion)
                .map(RuntimeEvaluateOutcome::Complete),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) if await_promise => {
                self.pending_internal_runtime_evaluates
                    .insert(call_id, response_rx);
                Ok(RuntimeEvaluateOutcome::Pending(
                    PendingRuntimeEvaluateCall { call_id },
                ))
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                self.page_inspector
                    .cancel_internal_runtime_evaluate_response(call_id);
                Err(anyhow!(
                    "internal Runtime.evaluate `{call_id}` produced no synchronous response"
                ))
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => Err(anyhow!(
                "internal Runtime.evaluate `{call_id}` response channel closed"
            )),
        }
    }

    pub(super) fn poll_pending_runtime_evaluate(
        &mut self,
        pending: PendingRuntimeEvaluateCall,
    ) -> Result<RuntimeEvaluateOutcome> {
        let Some(response_rx) = self
            .pending_internal_runtime_evaluates
            .get_mut(&pending.call_id)
        else {
            return Err(anyhow!(
                "internal Runtime.evaluate `{}` is no longer pending",
                pending.call_id
            ));
        };
        match response_rx.try_recv() {
            Ok(completion) => {
                self.pending_internal_runtime_evaluates
                    .remove(&pending.call_id);
                self.runtime_evaluate_payload_from_completion(completion)
                    .map(RuntimeEvaluateOutcome::Complete)
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                Ok(RuntimeEvaluateOutcome::Pending(pending))
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                self.pending_internal_runtime_evaluates
                    .remove(&pending.call_id);
                Err(anyhow!(
                    "internal Runtime.evaluate `{}` response channel closed",
                    pending.call_id
                ))
            }
        }
    }

    pub(super) fn cancel_pending_runtime_evaluate(&mut self, pending: PendingRuntimeEvaluateCall) {
        if self
            .pending_internal_runtime_evaluates
            .remove(&pending.call_id)
            .is_some()
        {
            self.page_inspector
                .cancel_internal_runtime_evaluate_response(pending.call_id);
        }
    }

    fn validate_runtime_evaluate_context(&self, execution_context_id: Option<i64>) -> Result<()> {
        let Some(execution_context_id) = execution_context_id else {
            return Ok(());
        };
        if self.runtime_observable_default_execution_context_id() == Some(execution_context_id)
            || self
                .page_isolated_world_contexts
                .has_execution_context_id(execution_context_id)
            || self
                .child_frame_realm_store
                .contains_key(&execution_context_id)
        {
            return Ok(());
        }
        Err(anyhow!(
            "unknown execution context `{execution_context_id}`"
        ))
    }

    fn next_internal_runtime_evaluate_call_id(&mut self) -> Result<i32> {
        let call_id = self.next_internal_runtime_evaluate_call_id;
        self.next_internal_runtime_evaluate_call_id =
            call_id.checked_add(1).filter(|next| *next > 0).unwrap_or(1);
        if self
            .pending_internal_runtime_evaluates
            .contains_key(&call_id)
        {
            return Err(anyhow!("internal Runtime.evaluate call id space exhausted"));
        }
        Ok(call_id)
    }

    fn require_completed_runtime_evaluate(
        &mut self,
        outcome: RuntimeEvaluateOutcome,
    ) -> Result<Value> {
        match outcome {
            RuntimeEvaluateOutcome::Complete(payload) => Ok(payload),
            RuntimeEvaluateOutcome::Pending(pending) => {
                self.cancel_pending_runtime_evaluate(pending);
                Err(anyhow!(
                    "internal Runtime.evaluate promise remained pending outside an owner continuation"
                ))
            }
        }
    }

    fn runtime_evaluate_payload_from_completion(
        &self,
        completion: RendererRuntimeInspectorAsyncCompletion,
    ) -> Result<Value> {
        let call_id = completion.call_id;
        let response = completion
            .output
            .into_protocol_response(call_id)
            .ok_or_else(|| anyhow!("internal Runtime.evaluate `{call_id}` returned no response"))?;
        if let Some(error) = response.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown inspector error");
            return Err(anyhow!(
                "internal Runtime.evaluate `{call_id}` failed: {message}"
            ));
        }
        let result = response
            .get("result")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                anyhow!("internal Runtime.evaluate `{call_id}` returned an invalid result")
            })?;
        if let Some(exception_details) = result.get("exceptionDetails") {
            let description = exception_details
                .pointer("/exception/description")
                .and_then(Value::as_str)
                .or_else(|| exception_details.get("text").and_then(Value::as_str))
                .unwrap_or("Uncaught");
            return Ok(json!({ "exception": description }));
        }
        result.get("result").cloned().ok_or_else(|| {
            anyhow!("internal Runtime.evaluate `{call_id}` returned no remote object")
        })
    }

    pub(super) fn exec_in_execution_context(
        &mut self,
        execution_context_id: i64,
        source: &str,
    ) -> Result<()> {
        if self
            .child_frame_realm_store
            .contains_key(&execution_context_id)
        {
            return self.exec_child_frame_source_script_job_for_execution_context_id(
                execution_context_id,
                FrameScriptJobKind::Eval,
                source,
            );
        }
        self.exec_in_isolated_context(execution_context_id, source)
    }

    fn runtime_binding_document_owner(
        &mut self,
        execution_context_id: Option<i64>,
    ) -> Result<FrameDocumentTaskOwner> {
        if execution_context_id.is_none()
            || execution_context_id == self.runtime_observable_default_execution_context_id()
        {
            return self
                .current_main_document_task_owner()
                .ok_or_else(|| anyhow!("Runtime binding has no current main document owner"));
        }

        let execution_context_id = execution_context_id.expect("checked execution context id");
        if let Some(world) = self
            .page_isolated_world_contexts
            .context(execution_context_id)
        {
            let owner = world.document_owner;
            if !self
                ._context_host
                .borrow()
                .document_task_owner_is_current(owner)
            {
                return Err(anyhow!(
                    "isolated execution context `{execution_context_id}` belongs to a retired document"
                ));
            }
            return Ok(owner);
        }

        self.prune_stale_child_default_execution_contexts();
        let child_handle = self
            .child_frame_realm_store
            .get(&execution_context_id)
            .map(|world| world.child_handle)
            .ok_or_else(|| anyhow!("unknown execution context `{execution_context_id}`"))?;
        self.child_frame_owner_realm_id_for_execution_context_id(execution_context_id)?;
        self._context_host
            .borrow()
            .current_child_document_task_owner(child_handle)
            .ok_or_else(|| {
                anyhow!(
                    "child execution context `{execution_context_id}` has no current document owner"
                )
            })
    }

    fn install_runtime_binding_in_default_context(&mut self, name: &str) -> Result<()> {
        let owner = self.runtime_binding_document_owner(None)?;
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context as *const _;
        let execution_context_id = self.default_or_initial_execution_context_id().unwrap_or(0);
        self.install_runtime_binding_in_context(context_ptr, execution_context_id, owner, name)
    }

    fn install_runtime_binding_in_execution_context(
        &mut self,
        execution_context_id: i64,
        name: &str,
    ) -> Result<()> {
        if execution_context_id == 0 {
            return self.install_runtime_binding_in_default_context(name);
        }
        if self
            .child_frame_realm_store
            .contains_key(&execution_context_id)
        {
            return self
                .install_runtime_binding_in_child_default_context(execution_context_id, name);
        }
        self.install_runtime_binding_in_isolated_context(execution_context_id, name)
    }

    fn install_runtime_binding_in_isolated_context(
        &mut self,
        execution_context_id: i64,
        name: &str,
    ) -> Result<()> {
        let owner = self.runtime_binding_document_owner(Some(execution_context_id))?;
        let context_ptr: *const v8::Global<v8::Context> = self
            .page_isolated_world_contexts
            .context(execution_context_id)
            .map(|world| &world.context as *const _)
            .ok_or_else(|| {
                anyhow!("unknown isolated execution context `{execution_context_id}`")
            })?;
        self.install_runtime_binding_in_context(context_ptr, execution_context_id, owner, name)
    }

    fn install_runtime_binding_in_child_default_context(
        &mut self,
        execution_context_id: i64,
        name: &str,
    ) -> Result<()> {
        let owner = self.runtime_binding_document_owner(Some(execution_context_id))?;
        let context_ptr =
            self.child_frame_realm_context_ptr_for_execution_context_id(execution_context_id)?;
        self.install_runtime_binding_in_context(context_ptr, execution_context_id, owner, name)
    }

    fn install_runtime_binding_in_context(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        execution_context_id: i64,
        owner: FrameDocumentTaskOwner,
        name: &str,
    ) -> Result<()> {
        self.with_context_scope_by_ptr(context_ptr, |scope, runtime_ptr| {
            let context_token = crate::native_bridge::current_runtime_observable_context_token(
                scope,
            )
            .ok_or_else(|| anyhow!("Runtime binding context has no runtime context token"))?;
            let execution_context = crate::native_bridge::RuntimeBindingExecutionContext::new(
                owner.local_window_id,
                context_token,
            );
            // SAFETY: `with_context_scope_by_ptr` keeps the context host alive and
            // invokes this closure synchronously without retaining `runtime_ptr`.
            if !unsafe { &mut *runtime_ptr }
                .register_runtime_binding_execution_context(execution_context, owner)
            {
                return Err(anyhow!(
                    "Runtime binding context belongs to a retired document owner"
                ));
            }
            let global = scope.get_current_context().global(scope);
            let key = v8_string(scope, name)
                .ok_or_else(|| anyhow!("failed to allocate runtime binding key `{name}`"))?;
            let data = build_runtime_binding_data(
                scope,
                runtime_ptr.cast::<std::ffi::c_void>(),
                key,
                execution_context_id,
                execution_context,
            )
            .map_err(|error| anyhow!("failed to declare runtime binding data: {error}"))?;
            let binding = v8::Function::builder(runtime_binding_callback)
                .data(data.into())
                .build(scope)
                .ok_or_else(|| anyhow!("failed to create runtime binding `{name}`"))?;
            global
                .define_own_property(
                    scope,
                    key.into(),
                    binding.into(),
                    v8::PropertyAttribute::DONT_ENUM,
                )
                .unwrap_or(false)
                .then_some(())
                .ok_or_else(|| anyhow!("failed to install runtime binding `{name}`"))?;
            Ok(())
        })
    }

    fn remove_runtime_binding_from_default_context(&mut self, name: &str) -> Result<()> {
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context as *const _;
        self.with_context_scope_by_ptr(context_ptr, |scope, _| {
            let global = scope.get_current_context().global(scope);
            let key = v8_string(scope, name)
                .ok_or_else(|| anyhow!("failed to allocate runtime binding key `{name}`"))?;
            let _ = global.delete(scope, key.into());
            Ok(())
        })
    }

    fn remove_runtime_binding_from_isolated_context(
        &mut self,
        execution_context_id: i64,
        name: &str,
    ) -> Result<()> {
        let context_ptr: *const v8::Global<v8::Context> = self
            .page_isolated_world_contexts
            .context(execution_context_id)
            .map(|world| &world.context as *const _)
            .ok_or_else(|| {
                anyhow!("unknown isolated execution context `{execution_context_id}`")
            })?;
        self.with_context_scope_by_ptr(context_ptr, |scope, _| {
            let global = scope.get_current_context().global(scope);
            let key = v8_string(scope, name)
                .ok_or_else(|| anyhow!("failed to allocate runtime binding key `{name}`"))?;
            let _ = global.delete(scope, key.into());
            Ok(())
        })
    }

    fn remove_runtime_binding_from_child_default_contexts(&mut self, name: &str) -> Result<()> {
        let context_ids = self
            .child_frame_realm_store
            .execution_context_ids()
            .collect::<Vec<_>>();
        for execution_context_id in context_ids {
            self.with_child_frame_realm_context_scope(execution_context_id, |scope, _| {
                let global = scope.get_current_context().global(scope);
                let key = v8_string(scope, name)
                    .ok_or_else(|| anyhow!("failed to allocate runtime binding key `{name}`"))?;
                let _ = global.delete(scope, key.into());
                Ok(())
            })?;
        }
        Ok(())
    }

    fn content_security_policy_script_element_request<'a>(
        &self,
        script: &'a PreparedScript,
    ) -> ContentSecurityPolicyScriptElementRequest<'a> {
        let parser_inserted_by_handle =
            script.host_script_handle.as_deref().is_some_and(|handle| {
                matches!(
                    self.document_runtime.script_handle_source(handle),
                    ScriptHandleSource::ParserOwned | ScriptHandleSource::DocumentWriteOwned
                )
            });
        let parser_inserted_by_node = self
            .document_runtime
            .dom_host()
            .node(script.node_id)
            .is_some_and(|node| node.flags().parser_created());
        ContentSecurityPolicyScriptElementRequest {
            nonce: script.fetch_metadata.nonce.as_deref(),
            integrity: script.fetch_metadata.integrity.as_deref(),
            parser_inserted: parser_inserted_by_handle || parser_inserted_by_node,
        }
    }

    pub(crate) async fn prepare_prepared_script_run(
        &mut self,
        loader: &ResourceRequestClient,
        script: &PreparedScript,
    ) -> std::result::Result<Option<PreparedScriptRunInput>, PreparedScriptExecutionError> {
        self.prepare_prepared_script_run_with_options(loader, script, true, None)
            .await
    }

    pub(crate) async fn prepare_prepared_script_run_without_blocker_wait(
        &mut self,
        loader: &ResourceRequestClient,
        script: &PreparedScript,
    ) -> std::result::Result<Option<PreparedScriptRunInput>, PreparedScriptExecutionError> {
        self.prepare_prepared_script_run_with_options(loader, script, false, None)
            .await
    }

    async fn prepare_prepared_script_run_with_options(
        &mut self,
        loader: &ResourceRequestClient,
        script: &PreparedScript,
        wait_for_blocking_stylesheets: bool,
        blocking_signatures_before: Option<
            &std::collections::HashSet<
                crate::stylesheet_blocking::DocumentBlockingStylesheetSignature,
            >,
        >,
    ) -> std::result::Result<Option<PreparedScriptRunInput>, PreparedScriptExecutionError> {
        if !self.prepared_script_is_live_for_execution(script) {
            return Ok(None);
        }
        debug!(
            url = %script.url,
            mode = ?script.mode,
            kind = ?script.kind,
            source_kind = ?script.source_kind,
            "execute_prepared_script_once loading source"
        );
        let load_started = Instant::now();
        let current_script = script
            .host_script_handle
            .as_deref()
            .and_then(|handle| self.document_runtime.resolve_host_script_handle(handle))
            .or(Some(script.node_id));
        let csp_script_request = self.content_security_policy_script_element_request(script);
        if script.source_kind == ScriptSourceKind::External
            && let Some(violation) = self
                .document_runtime
                .script_element_request_csp_report_only_violation_with_request(
                    &script.url,
                    csp_script_request,
                )
        {
            self.queue_content_security_policy_violation_event_best_effort(&violation);
        }
        if script.source_kind == ScriptSourceKind::External
            && let Some(violation) = self
                .document_runtime
                .script_element_request_csp_violation_with_request(&script.url, csp_script_request)
        {
            self.queue_content_security_policy_violation_event_best_effort(&violation);
            let message = format!(
                "Refused to load script `{}` because it violates the document Content Security Policy directive `{}`",
                script.url, violation.effective_directive
            );
            return Err(if script.kind == ScriptKind::Module {
                PreparedScriptExecutionError::from_top_level_module_source_load_failure(message)
            } else {
                PreparedScriptExecutionError::from_message(message)
            });
        }
        if wait_for_blocking_stylesheets
            && self
                .document_runtime
                .prepared_script_waits_for_blocking_stylesheets(script)
        {
            match blocking_signatures_before {
                Some(signatures) => {
                    self.document_runtime
                        .wait_for_document_owned_blocking_stylesheet_signatures(signatures.iter())
                        .await;
                }
                None => {
                    let completed_stylesheet_clients = self
                        .document_runtime
                        .wait_for_script_blockers_before(script.node_id)
                        .await;
                    self.settle_stylesheet_link_clients(completed_stylesheet_clients);
                }
            }
            self.record_ready_stylesheet_network_results();
        }
        if prepared_script_uses_external_module_graph(script) {
            debug!(
                url = %script.url,
                elapsed_ms = load_started.elapsed().as_millis(),
                "execute_prepared_script_once prepared external module graph"
            );
            if moli_trace::cdp_nav_timing_enabled() {
                tracing::info!(
                    target: "moli_cdp_nav_timing",
                    url = %script.url,
                    elapsed_ms = load_started.elapsed().as_millis(),
                    kind = ?script.kind,
                    mode = ?script.mode,
                    source_kind = ?script.source_kind,
                    stage = "renderer_prepared_script_external_module_graph_prepared",
                );
            }
            return Ok(Some(PreparedScriptRunInput {
                current_script,
                parser_write_insertion_point_active: false,
                body: PreparedScriptRunBody::ExternalModuleGraph,
            }));
        }
        let (source, source_bytes) = match &script.source {
            super::planning::ScriptSource::Loaded(source) => (source.clone(), None),
            super::planning::ScriptSource::LoadedBinary { source, bytes } => {
                (source.clone(), Some(bytes.clone()))
            }
            _ => {
                let document_character_set =
                    self.document_runtime.document_character_set().to_owned();
                let outcome =
                    super::planning::load_prepared_script_source_outcome_with_document_character_set(
                        script,
                        loader,
                        Some(&document_character_set),
                        None,
                    )
                    .await;
                if let Some(network_result) = outcome.network_result.as_deref() {
                    self._context_host
                        .borrow_mut()
                        .record_get_subresource_network_result_with_initiator(
                            None,
                            script.initiator_url.clone(),
                            script.url.clone(),
                            SubresourceResourceType::Script,
                            crate::types::SubresourceRequestInitiatorType::Parser,
                            network_result,
                        );
                }
                self.enforce_external_script_redirect_csp(
                    script,
                    outcome.network_result.as_deref(),
                )?;
                let source = outcome.source_result.map_err(|message| {
                    if script.kind == ScriptKind::Module {
                        PreparedScriptExecutionError::from_top_level_module_source_load_failure(
                            message,
                        )
                    } else {
                        PreparedScriptExecutionError::from_message(message)
                    }
                })?;
                (source, outcome.source_bytes)
            }
        };
        debug!(
            url = %script.url,
            source_len = source.len(),
            elapsed_ms = load_started.elapsed().as_millis(),
            "execute_prepared_script_once source loaded"
        );
        if moli_trace::cdp_nav_timing_enabled() {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url = %script.url,
                elapsed_ms = load_started.elapsed().as_millis(),
                source_len = source.len(),
                kind = ?script.kind,
                mode = ?script.mode,
                source_kind = ?script.source_kind,
                stage = "renderer_prepared_script_source_loaded",
            );
        }
        Ok(Some(PreparedScriptRunInput {
            current_script,
            parser_write_insertion_point_active: script.kind == ScriptKind::Classic
                && script.mode == ScriptMode::Normal,
            body: PreparedScriptRunBody::LoadedSource {
                source,
                source_bytes,
            },
        }))
    }

    fn enforce_external_script_redirect_csp(
        &mut self,
        script: &PreparedScript,
        network_result: Option<&std::result::Result<crate::types::NavigationResponse, String>>,
    ) -> std::result::Result<(), PreparedScriptExecutionError> {
        if script.source_kind != ScriptSourceKind::External {
            return Ok(());
        }
        let Some(Ok(response)) = network_result else {
            return Ok(());
        };
        if !response.redirected {
            return Ok(());
        }
        let redirect_status =
            crate::content_security_policy::ContentSecurityPolicyRedirectStatus::FollowedRedirect;
        let csp_script_request = self.content_security_policy_script_element_request(script);
        if let Some(violation) = self
            .document_runtime
            .script_element_request_csp_report_only_violation_with_redirect_status(
                &response.final_url,
                redirect_status,
                csp_script_request,
            )
        {
            self.queue_content_security_policy_violation_event_best_effort(&violation);
        }
        let Some(violation) = self
            .document_runtime
            .script_element_request_csp_violation_with_redirect_status(
                &response.final_url,
                redirect_status,
                csp_script_request,
            )
        else {
            return Ok(());
        };
        self.queue_content_security_policy_violation_event_best_effort(&violation);
        let message = format!(
            "Refused to load script `{}` because it violates the document Content Security Policy directive `{}`",
            response.final_url, violation.effective_directive
        );
        Err(if script.kind == ScriptKind::Module {
            PreparedScriptExecutionError::from_top_level_module_source_load_failure(message)
        } else {
            PreparedScriptExecutionError::from_message(message)
        })
    }

    pub(crate) async fn run_parser_owned_classic_script_without_blocker_wait(
        &mut self,
        loader: &ResourceRequestClient,
        script: &PreparedScript,
        execution_context: &ParserOwnedClassicScriptExecutionContext,
    ) -> ParserOwnedClassicScriptExecutionReport {
        let run_input = match self
            .prepare_prepared_script_run_without_blocker_wait(loader, script)
            .await
        {
            Ok(Some(run_input)) => run_input,
            Ok(None) => {
                return ParserOwnedClassicScriptExecutionReport::new(
                    Ok(()),
                    None,
                    ParserOwnedClassicScriptEvaluationSettlement::NotSettled,
                    PreparedScriptBodyActivity::NotEntered,
                );
            }
            Err(error) => {
                let script_element_event = self
                    .plan_parser_owned_external_classic_completion_event(
                        script,
                        ScriptEventKind::Error,
                    );
                tracing::debug!(
                    url = %script.url,
                    event_planned = script_element_event.is_some(),
                    error = error.message(),
                    "parser-owned classic preparation produced completion work"
                );
                return ParserOwnedClassicScriptExecutionReport::new(
                    Err(ParserOwnedClassicScriptExecutionError::new(
                        error.into_message(),
                    )),
                    script_element_event,
                    ParserOwnedClassicScriptEvaluationSettlement::NotSettled,
                    PreparedScriptBodyActivity::NotEntered,
                );
            }
        };
        let parser_bridge = match (
            run_input.current_script,
            run_input.parser_write_insertion_point_active,
            execution_context.parser_bridge(),
        ) {
            (Some(_), true, Some(bridge)) => Some(bridge.clone()),
            _ => None,
        };
        // XML parser-blocking scripts have a currentScript and use the same
        // execution/lifecycle coordinator, but XML has no HTML insertion point.
        // Absence of a controller therefore means document.write is inactive;
        // it must not suppress otherwise valid XHTML/SVG script execution.
        let parser_write_insertion_point_active =
            run_input.parser_write_insertion_point_active && parser_bridge.is_some();
        self.document_runtime
            .set_current_script_context(CurrentScriptContextSpec {
                handle: run_input.current_script,
                parser_write_insertion_point_active,
                parser_bridge,
            });
        let document_owner_before_run = self
            .current_main_document_task_owner()
            .expect("parser-owned classic execution requires a current main Document owner");
        let result = {
            let _parser_script_nesting = execution_context
                .is_parser_blocking()
                .then(|| self.document_runtime.enter_parser_script_nesting());
            self.execute_prepared_script_run_body(script, run_input.body)
                .await
        };
        self.document_runtime.clear_current_script_handle();
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(error) => {
                let script_element_event = self
                    .plan_parser_owned_external_classic_completion_event(
                        script,
                        ScriptEventKind::Error,
                    );
                tracing::debug!(
                    url = %script.url,
                    event_planned = script_element_event.is_some(),
                    error = error.message(),
                    "parser-owned classic execution produced completion work"
                );
                return ParserOwnedClassicScriptExecutionReport::new(
                    Err(ParserOwnedClassicScriptExecutionError::new(
                        error.into_message(),
                    )),
                    script_element_event,
                    ParserOwnedClassicScriptEvaluationSettlement::NotSettled,
                    PreparedScriptBodyActivity::Entered,
                );
            }
        };
        let body_activity = outcome.body_activity();
        match outcome {
            LoadedScriptExecutionOutcome::Completed(_) => {}
            LoadedScriptExecutionOutcome::CompletedModuleGraph(_)
            | LoadedScriptExecutionOutcome::SuspendedModuleFetches(_) => {
                let message = format!(
                    "parser-owned classic script `{}` produced a native ESM continuation",
                    script.url
                );
                let script_element_event = self
                    .plan_parser_owned_external_classic_completion_event(
                        script,
                        ScriptEventKind::Error,
                    );
                return ParserOwnedClassicScriptExecutionReport::new(
                    Err(ParserOwnedClassicScriptExecutionError::new(message)),
                    script_element_event,
                    ParserOwnedClassicScriptEvaluationSettlement::NotSettled,
                    body_activity,
                );
            }
        }
        if self.script_run_replaced_document(document_owner_before_run, script) {
            return ParserOwnedClassicScriptExecutionReport::new(
                Ok(()),
                None,
                ParserOwnedClassicScriptEvaluationSettlement::Settled,
                body_activity,
            );
        }
        let script_element_event =
            self.plan_parser_owned_external_classic_completion_event(script, ScriptEventKind::Load);
        tracing::debug!(
            url = %script.url,
            event_planned = script_element_event.is_some(),
            "parser-owned classic execution is awaiting owner completion"
        );
        ParserOwnedClassicScriptExecutionReport::new(
            Ok(()),
            script_element_event,
            ParserOwnedClassicScriptEvaluationSettlement::Settled,
            body_activity,
        )
    }

    async fn execute_prepared_script_once(
        &mut self,
        loader: &ResourceRequestClient,
        script: &PreparedScript,
    ) -> std::result::Result<bool, PreparedScriptExecutionError> {
        self.execute_prepared_script_once_with_blocking_signatures(loader, script, None)
            .await
    }

    async fn execute_prepared_script_once_with_blocking_signatures(
        &mut self,
        loader: &ResourceRequestClient,
        script: &PreparedScript,
        blocking_signatures_before: Option<
            &std::collections::HashSet<
                crate::stylesheet_blocking::DocumentBlockingStylesheetSignature,
            >,
        >,
    ) -> std::result::Result<bool, PreparedScriptExecutionError> {
        let Some(run_input) = self
            .prepare_prepared_script_run_with_options(
                loader,
                script,
                true,
                blocking_signatures_before,
            )
            .await?
        else {
            return Ok(false);
        };
        self.document_runtime
            .set_current_script_context(CurrentScriptContextSpec {
                handle: run_input.current_script,
                parser_write_insertion_point_active: run_input.parser_write_insertion_point_active,
                parser_bridge: None,
            });
        let result = self
            .execute_prepared_script_run_body(script, run_input.body)
            .await;
        self.document_runtime.clear_current_script_handle();
        match result? {
            LoadedScriptExecutionOutcome::Completed(_) => Ok(true),
            LoadedScriptExecutionOutcome::CompletedModuleGraph(_)
            | LoadedScriptExecutionOutcome::SuspendedModuleFetches(_) => {
                Err(PreparedScriptExecutionError::from_message(format!(
                    "runtime module script `{}` produced a native ESM continuation outside page-task execution",
                    script.url
                )))
            }
        }
    }

    async fn execute_prepared_script_run_body(
        &mut self,
        script: &PreparedScript,
        body: PreparedScriptRunBody,
    ) -> std::result::Result<LoadedScriptExecutionOutcome, PreparedScriptExecutionError> {
        match body {
            PreparedScriptRunBody::LoadedSource {
                source,
                source_bytes,
            } => {
                self.execute_loaded_prepared_script_source(script, &source, source_bytes.as_deref())
                    .await
            }
            PreparedScriptRunBody::ExternalModuleGraph => {
                self.execute_external_prepared_module_script_graph(script)
                    .await
            }
        }
    }

    async fn execute_external_prepared_module_script_graph(
        &mut self,
        script: &PreparedScript,
    ) -> std::result::Result<LoadedScriptExecutionOutcome, PreparedScriptExecutionError> {
        let completion_owner = self.completion_owner_for_prepared_module_script(script);
        match execute_external_module_script_graph(
            self,
            &script.url,
            &script.initiator_url,
            &script.fetch_metadata,
            completion_owner,
        )
        .await
        .map_err(PreparedScriptExecutionError::from_module_load_error)?
        {
            ModuleScriptExecutionOutcome::CompletedModuleGraph(graph) => {
                Ok(LoadedScriptExecutionOutcome::CompletedModuleGraph(graph))
            }
            ModuleScriptExecutionOutcome::SuspendedModuleFetches(continuation) => Ok(
                LoadedScriptExecutionOutcome::SuspendedModuleFetches(continuation),
            ),
        }
    }

    pub(crate) async fn execute_loaded_prepared_script_source(
        &mut self,
        script: &PreparedScript,
        source: &str,
        source_bytes: Option<&[u8]>,
    ) -> std::result::Result<LoadedScriptExecutionOutcome, PreparedScriptExecutionError> {
        let inline_source = if script.source_kind == ScriptSourceKind::Inline {
            let request = self.content_security_policy_script_element_request(script);
            let Some(source) =
                self.inline_script_element_source_for_execution(script.node_id, source, request)
            else {
                return Ok(LoadedScriptExecutionOutcome::Completed(
                    PreparedScriptBodyActivity::NotEntered,
                ));
            };
            Some(source)
        } else {
            None
        };
        let source = inline_source.as_deref().unwrap_or(source);
        let selector_before = self.document_runtime.selector_debug_snapshot();
        let started = Instant::now();
        debug!(url = %script.url, "execute_prepared_script_once executing source");
        self.script_execution_memory.record(script, source.len());
        Self::reset_dom_binding_trace_window();
        let dom_binding_source_started =
            moli_trace::dom_binding_timing_enabled().then(Instant::now);
        let completion_owner = self.completion_owner_for_prepared_module_script(script);
        let result = self
            .execute_loaded_script(
                script.kind,
                script.source_kind,
                source,
                source_bytes,
                &script.url,
                &script.base_url,
                &script.initiator_url,
                script.node_id,
                &script.fetch_metadata,
                completion_owner,
            )
            .await;
        if let Some(started) = dom_binding_source_started {
            Self::emit_dom_binding_trace_window(
                "renderer_prepared_script_dom_binding_summary",
                "source_eval",
                Some(&script.url),
                started.elapsed(),
            );
        }
        let selector_after = self.document_runtime.selector_debug_snapshot();
        debug!(
            url = %script.url,
            result = ?result.as_ref().map(|_| ()).map_err(|error| error.message()),
            elapsed_ms = started.elapsed().as_millis(),
            source_len = source.len(),
            kind = ?script.kind,
            mode = ?script.mode,
            source_kind = ?script.source_kind,
            query_selector_delta = selector_after.query_selector.saturating_sub(selector_before.query_selector),
            query_selector_all_delta = selector_after.query_selector_all.saturating_sub(selector_before.query_selector_all),
            matches_delta = selector_after.matches.saturating_sub(selector_before.matches),
            closest_delta = selector_after.closest.saturating_sub(selector_before.closest),
            "execute_prepared_script_once finished"
        );
        if moli_trace::cdp_nav_timing_enabled() {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url = %script.url,
                result = ?result.as_ref().map(|_| ()).map_err(|error| error.message()),
                elapsed_ms = started.elapsed().as_millis(),
                source_len = source.len(),
                kind = ?script.kind,
                mode = ?script.mode,
                source_kind = ?script.source_kind,
                query_selector_delta = selector_after
                    .query_selector
                    .saturating_sub(selector_before.query_selector),
                query_selector_all_delta = selector_after
                    .query_selector_all
                    .saturating_sub(selector_before.query_selector_all),
                matches_delta = selector_after.matches.saturating_sub(selector_before.matches),
                closest_delta = selector_after.closest.saturating_sub(selector_before.closest),
                stage = "renderer_prepared_script_source_done",
            );
        }
        result
    }

    async fn execute_loaded_script(
        &mut self,
        kind: ScriptKind,
        source_kind: ScriptSourceKind,
        source: &str,
        source_bytes: Option<&[u8]>,
        script_url: &Url,
        script_base_url: &Url,
        initiator_url: &Url,
        script_node_id: NodeId,
        fetch_metadata: &crate::planning::ScriptFetchMetadata,
        completion_owner: ModuleScriptCompletionOwner,
    ) -> std::result::Result<LoadedScriptExecutionOutcome, PreparedScriptExecutionError> {
        match kind {
            ScriptKind::Classic => {
                let provenance =
                    CompiledStringProvenance::new(script_url.clone(), script_base_url.clone());
                let result = self.exec_in_enclosing_script_turn_with_provenance(
                    source,
                    &provenance,
                    if self
                        .document_runtime
                        .parser_script_start_line(script_node_id)
                        .is_some_and(|line| line > 1)
                        && script_url.as_str() == self.document_runtime.document_url().as_str()
                    {
                        self.document_runtime
                            .parser_script_start_line(script_node_id)
                            .map(|line| line.saturating_sub(1).min(i32::MAX as u64) as i32)
                            .unwrap_or(0)
                    } else {
                        0
                    },
                    fetch_metadata.nonce.as_deref(),
                    true,
                );
                match result {
                    Ok(()) => Ok(LoadedScriptExecutionOutcome::Completed(
                        PreparedScriptBodyActivity::Entered,
                    )),
                    Err(eval_exec::RawScriptExecutionError::Exception { report, .. }) => {
                        self.report_classic_script_exception_and_finish_evaluation_best_effort(
                            &report,
                        );
                        Ok(LoadedScriptExecutionOutcome::Completed(
                            PreparedScriptBodyActivity::Entered,
                        ))
                    }
                    Err(error) => Err(PreparedScriptExecutionError::from_entered_script_message(
                        error.into_anyhow().to_string(),
                    )),
                }
            }
            ScriptKind::ImportMap => {
                let _ = script_node_id;
                register_import_map_source(self, source)
                    .map_err(PreparedScriptExecutionError::from_message)?;
                Ok(LoadedScriptExecutionOutcome::Completed(
                    PreparedScriptBodyActivity::NotEntered,
                ))
            }
            ScriptKind::Module => {
                let module_source =
                    module_script_source_for_execution(script_url, source, source_bytes)
                        .map_err(PreparedScriptExecutionError::from_message)?;
                match execute_module_script_source(
                    self,
                    module_source,
                    script_base_url,
                    initiator_url,
                    fetch_metadata,
                    source_kind == ScriptSourceKind::External,
                    completion_owner,
                )
                .await
                .map_err(PreparedScriptExecutionError::from_module_load_error)?
                {
                    ModuleScriptExecutionOutcome::CompletedModuleGraph(graph) => {
                        Ok(LoadedScriptExecutionOutcome::CompletedModuleGraph(graph))
                    }
                    ModuleScriptExecutionOutcome::SuspendedModuleFetches(continuation) => Ok(
                        LoadedScriptExecutionOutcome::SuspendedModuleFetches(continuation),
                    ),
                }
            }
            ScriptKind::DataBlock => Err(PreparedScriptExecutionError::from_message(
                "data block should have been skipped",
            )),
        }
    }

    fn inline_script_element_source_for_execution(
        &mut self,
        node_id: DomHandle,
        source: &str,
        request: ContentSecurityPolicyScriptElementRequest<'_>,
    ) -> Option<String> {
        let context_host = self._context_host.clone();
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                // SAFETY: context_ptr points to self.page_default_context, which
                // remains live for this non-escaping isolate closure.
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let host_ptr: *mut JsContextHost = (*context_host).as_ptr();
                Ok(
                    crate::native_bridge::element::inline_script_source_for_execution(
                        scope, host_ptr, node_id, source, request,
                    ),
                )
            })
            .ok()
            .flatten()
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

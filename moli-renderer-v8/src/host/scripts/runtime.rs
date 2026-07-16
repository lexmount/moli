use super::{
    HostScriptScheduler, PreparedScriptElementStart, RuntimeScriptPreparationContext,
    RuntimeScriptStartDecision, ScriptElementLoader, ScriptElementLoaderOptions, ScriptEventKind,
    ScriptEventTask, ScriptHandleSource, ScriptStartCommitKind,
};
#[cfg(test)]
use crate::types::ScriptSourceKind;
use crate::{
    dom::native::{DomHost, NativeNodeId},
    frame_owner_model::MainDocumentScriptLoadDelayKind,
    {
        host::{
            EventTargetHandle, HostDocumentState, HostEventTargetRegistry, dispatch_host_event,
        },
        native_bridge::JsContextHost,
    },
};
use tracing::debug;

use super::RuntimeScriptAdmissionPayload;

#[derive(Debug, Clone)]
pub(crate) struct PreparedRuntimeScriptStart {
    node: NativeNodeId,
    preparation: RuntimeScriptPreparationContext,
    pub(super) decision: RuntimeScriptStartDecision,
}

#[derive(Debug)]
pub(crate) struct RuntimeScriptStartPlan {
    host_script_handle: String,
    prepared: PreparedRuntimeScriptStart,
}

impl RuntimeScriptStartPlan {
    pub(crate) fn requires_runtime_admission(&self) -> bool {
        matches!(
            self.prepared.decision,
            RuntimeScriptStartDecision::Queue { .. }
                | RuntimeScriptStartDecision::QueueFailed { .. }
        )
    }

    pub(crate) fn load_delay_kind(&self) -> Option<MainDocumentScriptLoadDelayKind> {
        let kind = match &self.prepared.decision {
            RuntimeScriptStartDecision::Queue { kind, .. }
            | RuntimeScriptStartDecision::QueueFailed { kind, .. } => *kind,
            _ => return None,
        };
        Some(if kind == crate::types::ScriptKind::Module {
            MainDocumentScriptLoadDelayKind::Module
        } else {
            MainDocumentScriptLoadDelayKind::Classic
        })
    }
}

#[derive(Debug)]
pub(crate) struct RuntimeScriptStartReservation {
    node: NativeNodeId,
    host_script_handle: String,
    commit_kind: ScriptStartCommitKind,
}

#[derive(Debug)]
pub(crate) enum PreparedRuntimeScriptStartCommit {
    Noop,
    InlineClassic {
        node: NativeNodeId,
        host_script_handle: String,
        source: String,
    },
    InlineImportMap {
        node: NativeNodeId,
        base_url: url::Url,
        source: String,
    },
    Admission {
        reservation: RuntimeScriptStartReservation,
        payload: Box<RuntimeScriptAdmissionPayload>,
    },
}

#[derive(Debug)]
pub(crate) struct CommittedInlineClassicScript {
    node: NativeNodeId,
    host_script_handle: String,
    source: String,
}

impl CommittedInlineClassicScript {
    pub(crate) fn new(node: NativeNodeId, host_script_handle: String, source: String) -> Self {
        Self {
            node,
            host_script_handle,
            source,
        }
    }

    pub(crate) fn into_parts(self) -> (NativeNodeId, String, String) {
        (self.node, self.host_script_handle, self.source)
    }
}

impl PreparedRuntimeScriptStart {
    #[cfg(test)]
    pub(crate) fn analyze(
        dom_host: &mut DomHost,
        document: &HostDocumentState,
        node: NativeNodeId,
    ) -> PreparedRuntimeScriptStart {
        Self::analyze_with_loader_options(
            dom_host,
            document,
            node,
            ScriptElementLoaderOptions::with_scripting_enabled(true),
        )
    }

    fn analyze_with_loader_options(
        dom_host: &mut DomHost,
        document: &HostDocumentState,
        node: NativeNodeId,
        options: ScriptElementLoaderOptions,
    ) -> PreparedRuntimeScriptStart {
        let PreparedScriptElementStart {
            preparation,
            decision,
        } = ScriptElementLoader::prepare(dom_host, document, node, options);
        PreparedRuntimeScriptStart {
            node,
            preparation,
            decision,
        }
    }

    pub(crate) fn needs_commit(&self) -> bool {
        !matches!(
            self.decision,
            RuntimeScriptStartDecision::Skip {
                commit_start: false,
                ..
            }
        )
    }

    pub(crate) fn into_plan(self, host_script_handle: impl Into<String>) -> RuntimeScriptStartPlan {
        RuntimeScriptStartPlan {
            host_script_handle: host_script_handle.into(),
            prepared: self,
        }
    }

    #[cfg(test)]
    pub(crate) fn execute(
        self,
        dom_host: &mut DomHost,
        scripts: &mut HostScriptScheduler,
        host_script_handle: &str,
    ) -> std::result::Result<Option<String>, String> {
        if self.needs_commit() && !scripts.reserve_script_start(host_script_handle, self.node) {
            return Ok(None);
        }

        self.execute_after_reservation(dom_host, scripts, host_script_handle)
    }

    #[cfg(test)]
    fn execute_after_reservation(
        self,
        dom_host: &mut DomHost,
        scripts: &mut HostScriptScheduler,
        host_script_handle: &str,
    ) -> std::result::Result<Option<String>, String> {
        let PreparedRuntimeScriptStart {
            node,
            preparation,
            decision,
        } = self;

        match decision {
            RuntimeScriptStartDecision::Skip { commit_start, .. } => {
                if !commit_start {
                    scripts.cancel_script_start(host_script_handle, node);
                    return Ok(None);
                }
                if !commit_runtime_script_start(
                    dom_host,
                    scripts,
                    node,
                    host_script_handle,
                    ScriptStartCommitKind::Skip,
                ) {
                    scripts.cancel_script_start(host_script_handle, node);
                    return Ok(None);
                }
                Ok(None)
            }
            RuntimeScriptStartDecision::ExecuteInlineClassic { source } => {
                if !commit_runtime_script_start(
                    dom_host,
                    scripts,
                    node,
                    host_script_handle,
                    ScriptStartCommitKind::ExecuteInline,
                ) {
                    scripts.cancel_script_start(host_script_handle, node);
                    return Ok(None);
                }
                debug!(
                    node = ?node,
                    host_script_handle,
                    source_len = source.len(),
                    "preparing inline classic script"
                );
                Ok(Some(source))
            }
            RuntimeScriptStartDecision::RegisterImportMap { source } => {
                if !commit_runtime_script_start(
                    dom_host,
                    scripts,
                    node,
                    host_script_handle,
                    ScriptStartCommitKind::RegisterImportMap,
                ) {
                    scripts.cancel_script_start(host_script_handle, node);
                    return Ok(None);
                }
                scripts.register_dynamic_import_map(&preparation.base_url, &source);
                Ok(None)
            }
            RuntimeScriptStartDecision::RejectExternalImportMap => {
                if !commit_runtime_script_start(
                    dom_host,
                    scripts,
                    node,
                    host_script_handle,
                    ScriptStartCommitKind::RejectImportMap,
                ) {
                    scripts.cancel_script_start(host_script_handle, node);
                    return Ok(None);
                }
                scripts.enqueue_script_event_lifecycle_work(
                    ScriptEventKind::Error,
                    host_script_handle,
                );
                Ok(None)
            }
            RuntimeScriptStartDecision::Queue {
                source,
                kind,
                mode,
                source_kind,
            } => {
                if source_kind == ScriptSourceKind::External {
                    debug!(
                        node = ?node,
                        host_script_handle,
                        source,
                        mode = ?mode,
                        kind = ?kind,
                        "queueing external dynamic script"
                    );
                    if let Err(error) = scripts.queue_dynamic_script_for_node(
                        &preparation,
                        node,
                        host_script_handle,
                        &source,
                        source_kind,
                        kind,
                        mode,
                    ) {
                        scripts.cancel_script_start(host_script_handle, node);
                        return Err(error);
                    }
                    if !commit_runtime_script_start(
                        dom_host,
                        scripts,
                        node,
                        host_script_handle,
                        ScriptStartCommitKind::Queue,
                    ) {
                        scripts.cancel_script_start(host_script_handle, node);
                        return Ok(None);
                    }
                    return Ok(None);
                }

                if !commit_runtime_script_start(
                    dom_host,
                    scripts,
                    node,
                    host_script_handle,
                    ScriptStartCommitKind::Queue,
                ) {
                    scripts.cancel_script_start(host_script_handle, node);
                    return Ok(None);
                }
                debug!(
                    node = ?node,
                    host_script_handle,
                    source_len = source.len(),
                    mode = ?mode,
                    kind = ?kind,
                    "queueing inline dynamic script"
                );
                if let Err(error) = scripts.queue_dynamic_script_for_node(
                    &preparation,
                    node,
                    host_script_handle,
                    &source,
                    source_kind,
                    kind,
                    mode,
                ) {
                    scripts.cancel_script_start(host_script_handle, node);
                    let _ = dom_host.set_script_already_started(node, false);
                    return Err(error);
                }
                Ok(None)
            }
            RuntimeScriptStartDecision::QueueFailed {
                source,
                kind,
                mode,
                source_kind,
                message,
            } => {
                debug!(
                    node = ?node,
                    host_script_handle,
                    source,
                    mode = ?mode,
                    kind = ?kind,
                    message,
                    "queueing failed dynamic script start"
                );
                if let Err(error) = scripts.queue_failed_dynamic_script(
                    &preparation,
                    host_script_handle,
                    &source,
                    source_kind,
                    kind,
                    mode,
                    &message,
                ) {
                    scripts.cancel_script_start(host_script_handle, node);
                    return Err(error);
                }
                if !commit_runtime_script_start(
                    dom_host,
                    scripts,
                    node,
                    host_script_handle,
                    ScriptStartCommitKind::QueueFailed,
                ) {
                    scripts.cancel_script_start(host_script_handle, node);
                    return Ok(None);
                }
                Ok(None)
            }
        }
    }
}

pub(crate) fn prepare_runtime_script_start_commit(
    dom_host: &mut DomHost,
    scripts: &mut HostScriptScheduler,
    plan: RuntimeScriptStartPlan,
) -> std::result::Result<PreparedRuntimeScriptStartCommit, String> {
    let RuntimeScriptStartPlan {
        host_script_handle,
        prepared,
    } = plan;
    scripts.register_script_handle_with_source(
        &host_script_handle,
        prepared.node,
        ScriptHandleSource::RuntimeOwned,
    );
    if prepared.needs_commit() && !scripts.reserve_script_start(&host_script_handle, prepared.node)
    {
        return Ok(PreparedRuntimeScriptStartCommit::Noop);
    }

    let PreparedRuntimeScriptStart {
        node,
        preparation,
        decision,
    } = prepared;

    match decision {
        RuntimeScriptStartDecision::Skip { commit_start, .. } => {
            if commit_start {
                let _ = finish_local_runtime_script_start(
                    dom_host,
                    scripts,
                    node,
                    &host_script_handle,
                    ScriptStartCommitKind::Skip,
                );
            }
            Ok(PreparedRuntimeScriptStartCommit::Noop)
        }
        RuntimeScriptStartDecision::ExecuteInlineClassic { source } => {
            if !finish_local_runtime_script_start(
                dom_host,
                scripts,
                node,
                &host_script_handle,
                ScriptStartCommitKind::ExecuteInline,
            ) {
                return Ok(PreparedRuntimeScriptStartCommit::Noop);
            }
            Ok(PreparedRuntimeScriptStartCommit::InlineClassic {
                node,
                host_script_handle,
                source,
            })
        }
        RuntimeScriptStartDecision::RegisterImportMap { source } => {
            if !finish_local_runtime_script_start(
                dom_host,
                scripts,
                node,
                &host_script_handle,
                ScriptStartCommitKind::RegisterImportMap,
            ) {
                return Ok(PreparedRuntimeScriptStartCommit::Noop);
            }
            Ok(PreparedRuntimeScriptStartCommit::InlineImportMap {
                node,
                base_url: preparation.base_url,
                source,
            })
        }
        RuntimeScriptStartDecision::RejectExternalImportMap => {
            if finish_local_runtime_script_start(
                dom_host,
                scripts,
                node,
                &host_script_handle,
                ScriptStartCommitKind::RejectImportMap,
            ) {
                scripts.enqueue_script_event_lifecycle_work(
                    ScriptEventKind::Error,
                    &host_script_handle,
                );
            }
            Ok(PreparedRuntimeScriptStartCommit::Noop)
        }
        RuntimeScriptStartDecision::Queue {
            source,
            kind,
            mode,
            source_kind,
        } => {
            let script = match scripts.prepare_dynamic_script(
                &preparation,
                node,
                &host_script_handle,
                &source,
                source_kind,
                kind,
                mode,
            ) {
                Ok(script) => script,
                Err(error) => {
                    scripts.cancel_script_start(&host_script_handle, node);
                    return Err(error);
                }
            };
            Ok(PreparedRuntimeScriptStartCommit::Admission {
                reservation: RuntimeScriptStartReservation {
                    node,
                    host_script_handle,
                    commit_kind: ScriptStartCommitKind::Queue,
                },
                payload: Box::new(RuntimeScriptAdmissionPayload::Script(script)),
            })
        }
        RuntimeScriptStartDecision::QueueFailed {
            source,
            kind,
            mode,
            source_kind,
            message,
        } => {
            let node_id = scripts.next_virtual_node_id();
            let failed = match scripts.prepare_failed_dynamic_script(
                &preparation,
                node_id,
                &host_script_handle,
                &source,
                source_kind,
                kind,
                mode,
                &message,
            ) {
                Ok(failed) => failed,
                Err(error) => {
                    scripts.cancel_script_start(&host_script_handle, node);
                    return Err(error);
                }
            };
            Ok(PreparedRuntimeScriptStartCommit::Admission {
                reservation: RuntimeScriptStartReservation {
                    node,
                    host_script_handle,
                    commit_kind: ScriptStartCommitKind::QueueFailed,
                },
                payload: Box::new(RuntimeScriptAdmissionPayload::Failed(failed)),
            })
        }
    }
}

fn finish_local_runtime_script_start(
    dom_host: &mut DomHost,
    scripts: &mut HostScriptScheduler,
    node: NativeNodeId,
    host_script_handle: &str,
    kind: ScriptStartCommitKind,
) -> bool {
    if commit_runtime_script_start(dom_host, scripts, node, host_script_handle, kind) {
        true
    } else {
        scripts.cancel_script_start(host_script_handle, node);
        false
    }
}

pub(crate) fn finish_runtime_script_start_admission(
    dom_host: &mut DomHost,
    scripts: &mut HostScriptScheduler,
    reservation: RuntimeScriptStartReservation,
) {
    assert!(
        commit_runtime_script_start(
            dom_host,
            scripts,
            reservation.node,
            &reservation.host_script_handle,
            reservation.commit_kind,
        ),
        "published runtime script admission must retain its exact start reservation"
    );
}

pub(crate) fn cancel_runtime_script_start_admission(
    scripts: &mut HostScriptScheduler,
    reservation: RuntimeScriptStartReservation,
) {
    scripts.cancel_script_start(&reservation.host_script_handle, reservation.node);
}

pub(super) fn commit_runtime_script_start(
    dom_host: &mut DomHost,
    scripts: &mut HostScriptScheduler,
    node: NativeNodeId,
    host_script_handle: &str,
    kind: ScriptStartCommitKind,
) -> bool {
    if !scripts.finish_script_start(host_script_handle, node, kind) {
        return false;
    }
    let _ = dom_host.set_script_already_started(node, true);
    true
}

/// Holds the per-element start lane while a Trusted Types default policy runs.
///
/// Policy callbacks can mutate the connected script and synchronously trigger
/// another preparation attempt. Reserving before entering JavaScript keeps
/// that nested attempt from preparing the same element a second time. The
/// reservation is released before the normal plan/commit pipeline resumes.
#[derive(Debug)]
pub(crate) struct RuntimeScriptTextPreparationReservation {
    node: NativeNodeId,
    host_script_handle: String,
}

impl RuntimeScriptTextPreparationReservation {
    pub(crate) fn begin(
        dom_host: &DomHost,
        scripts: &mut HostScriptScheduler,
        node: NativeNodeId,
        host_script_handle: &str,
    ) -> Option<Self> {
        let can_prepare = dom_host.is_connected(node)
            && dom_host
                .node(node)
                .and_then(|node| node.as_element())
                .is_some_and(|element| {
                    element.is_script_element() && !element.script_already_started()
                });
        if !can_prepare {
            return None;
        }
        scripts.register_script_handle_with_source(
            host_script_handle,
            node,
            ScriptHandleSource::RuntimeOwned,
        );
        scripts
            .reserve_script_start(host_script_handle, node)
            .then(|| Self {
                node,
                host_script_handle: host_script_handle.to_owned(),
            })
    }

    pub(crate) fn release(self, scripts: &mut HostScriptScheduler) {
        scripts.cancel_script_start(&self.host_script_handle, self.node);
    }
}

pub(crate) fn begin_prepared_document_write_script_start(
    dom_host: &mut DomHost,
    scripts: &mut HostScriptScheduler,
    node: NativeNodeId,
    host_script_handle: &str,
    execute_inline: bool,
) -> bool {
    scripts.register_script_handle_with_source(
        host_script_handle,
        node,
        ScriptHandleSource::DocumentWriteOwned,
    );
    if !scripts.reserve_script_start(host_script_handle, node) {
        return false;
    }
    let kind = if execute_inline {
        ScriptStartCommitKind::ExecuteInline
    } else {
        ScriptStartCommitKind::ExecutePrepared
    };
    commit_runtime_script_start(dom_host, scripts, node, host_script_handle, kind)
}

#[cfg(test)]
pub(crate) fn prepare_script_start(
    dom_host: &mut DomHost,
    document: &HostDocumentState,
    scripts: &mut HostScriptScheduler,
    node: NativeNodeId,
    host_script_handle: &str,
) -> std::result::Result<Option<String>, String> {
    if !dom_host.is_connected(node) || !dom_host.is_script_element(node) {
        return Ok(None);
    }
    scripts.register_script_handle_with_source(
        host_script_handle,
        node,
        ScriptHandleSource::RuntimeOwned,
    );
    PreparedRuntimeScriptStart::analyze(dom_host, document, node).execute(
        dom_host,
        scripts,
        host_script_handle,
    )
}

pub(crate) fn plan_script_start(
    dom_host: &mut DomHost,
    document: &HostDocumentState,
    node: NativeNodeId,
    host_script_handle: &str,
    options: ScriptElementLoaderOptions,
) -> Option<RuntimeScriptStartPlan> {
    if !dom_host.is_connected(node)
        || !dom_host.is_script_element(node)
        || dom_host.owner_document_handle(node) != Some(dom_host.document_handle())
    {
        return None;
    }
    Some(
        PreparedRuntimeScriptStart::analyze_with_loader_options(dom_host, document, node, options)
            .into_plan(host_script_handle),
    )
}

pub(crate) fn dispatch_script_event(
    dom_host: &DomHost,
    scripts: &HostScriptScheduler,
    events: &mut HostEventTargetRegistry,
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    task: &ScriptEventTask,
) -> std::result::Result<(), String> {
    let Some(script_handle) = scripts.script_handle_target(&task.handle) else {
        debug!(
            host_script_handle = task.handle.as_str(),
            event_kind = ?task.kind,
            "refusing to dispatch script event for unregistered handle"
        );
        debug_assert!(false, "script event dispatch requires a registered handle");
        return Err(format!(
            "script event dispatch requires registered handle `{}`",
            task.handle
        ));
    };

    let node_name = dom_host
        .node(script_handle)
        .map(|node| node.node_name())
        .unwrap_or_else(|| "<missing>".to_owned());
    let src = dom_host.get_attribute(script_handle, "src");
    let type_attr = dom_host.get_attribute(script_handle, "type");
    let id_attr = dom_host.get_attribute(script_handle, "id");
    let snippet = dom_host
        .dom()
        .outer_html(script_handle)
        .map(|html| {
            let collapsed = html.split_whitespace().collect::<Vec<_>>().join(" ");
            if collapsed.chars().count() > 240 {
                collapsed.chars().take(240).collect::<String>() + "..."
            } else {
                collapsed
            }
        })
        .unwrap_or_default();
    debug!(
        host_script_handle = task.handle.as_str(),
        event_kind = ?task.kind,
        ?script_handle,
        node_name,
        src = src.as_deref().unwrap_or(""),
        type_attr = type_attr.as_deref().unwrap_or(""),
        id_attr = id_attr.as_deref().unwrap_or(""),
        snippet,
        "dispatch script event"
    );

    dispatch_host_event(
        events,
        scope,
        host_ptr,
        EventTargetHandle::Node(script_handle),
        EventTargetHandle::Node(script_handle),
        task.event_name(),
        false,
        false,
    )?;

    if let Some(document_element) = dom_host.document_element_handle() {
        dispatch_host_event(
            events,
            scope,
            host_ptr,
            EventTargetHandle::Node(document_element),
            EventTargetHandle::Node(script_handle),
            task.event_name(),
            false,
            false,
        )?;
    }

    Ok(())
}

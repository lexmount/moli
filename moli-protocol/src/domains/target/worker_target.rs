//! Ordered protocol projection for renderer-owned worker targets.
//!
//! SharedWorker and ServiceWorker lifecycle events share one physical activity
//! slot so a capture batch retains source order through target attach, output,
//! detach, and destruction. They do not share lifecycle authority:
//! SharedWorker binds one stable target attachment, while ServiceWorker binds
//! independent stable-version, protocol-attachment, and per-run identities in
//! `conn::state`. ServiceWorker run-specific events already carry the opaque
//! identity created by the renderer authority; protocol capture projects that
//! exact run into a target/session identity before appending output. The
//! projection coordinator only validates and consumes the captured value.

use moli_core::page::{
    RendererDedicatedWorkerTargetEvent, RendererDedicatedWorkerTargetInfo,
    RendererRuntimeInspectorMessage, RendererServiceWorkerConsoleMessage,
    RendererServiceWorkerExceptionMessage, RendererServiceWorkerFetchDiagnostic,
    RendererServiceWorkerFetchDiagnosticResult, RendererServiceWorkerRunIdentity,
    RendererServiceWorkerTargetEvent, RendererServiceWorkerTargetInfo,
    RendererServiceWorkerVersionStatus, RendererSharedWorkerConsoleMessage,
    RendererSharedWorkerTargetEvent, RendererSharedWorkerTargetInfo, RuntimeConsoleMessageSnapshot,
    SubresourceRequestInitiatorType,
};
use moli_shared_worker::SharedWorkerInstanceId;
use serde_json::json;
use url::Url;

use crate::devtools_runtime::{
    DevToolsNetworkResourceType, DevToolsTargetKind, RuntimeExecutionContextsClearedEvent,
};
#[cfg(test)]
use crate::devtools_runtime::{DevToolsTargetInfo, RuntimeExecutionContextEvent};
use crate::{
    conn::{
        BackgroundProtocolEvent, CdpConnection, PreparedTargetAttach, PreparedTargetHostDelta,
        RendererPageResidenceIdentity, ServiceWorkerRuntimeExceptionSnapshot,
        ServiceWorkerTargetState, SharedWorkerTargetState, TargetPageResidenceIdentity,
        TargetServiceWorkerProtocolAttachmentIdentity,
        TargetServiceWorkerProtocolAttachmentRetirement, TargetServiceWorkerRunIdentity,
        TargetServiceWorkerRunRetirement, TargetServiceWorkerRuntimeAttachmentIdentity,
        TargetServiceWorkerVersionIdentity, TargetServiceWorkerVersionRetirement,
        TargetSessionDetachCleanupPlan, TargetSharedWorkerProtocolAttachmentIdentity,
        TargetSharedWorkerProtocolAttachmentRetirement, monotonic_timestamp_seconds,
    },
    domains::activity::{
        ProtocolOutputPayloads, ProtocolOutputProjectionContext, ProtocolOutputSink,
        ProtocolOutputSlot,
    },
    domains::observable_output::{
        console_message_added_background_event, console_message_level_and_text,
        runtime_console_api_called_background_event, runtime_console_message_type_and_text,
        runtime_exception_thrown_background_event,
    },
    domains::runtime::replay_shared_worker_runtime_bindings_for_session_async,
    domains::{network, service_worker},
};
#[cfg(test)]
use serde_json::Value;

use super::events;

#[derive(Debug, Default, PartialEq)]
pub(in crate::domains) struct TargetPreparedOutputs {
    worker_target_lifecycle_outputs: Vec<WorkerTargetLifecycleOutput>,
}

#[derive(Debug, PartialEq)]
enum WorkerTargetLifecycleOutput {
    DedicatedWorkerEvents {
        browser_context_id: String,
        renderer_instance_id: u64,
        target_id: String,
        events: Vec<crate::conn::BackgroundProtocolEvent>,
    },
    DedicatedWorkerConsoleMessages {
        browser_context_id: String,
        renderer_instance_id: u64,
        target_id: String,
        session_id: String,
        console_messages: Vec<RuntimeConsoleMessageSnapshot>,
        runtime_messages: Vec<RuntimeConsoleMessageSnapshot>,
        console_end: usize,
    },
    DedicatedWorkerCreated {
        browser_context_id: String,
        renderer_instance_id: u64,
        target_delta: PreparedTargetHostDelta,
    },
    DedicatedWorkerInfoChanged {
        browser_context_id: String,
        renderer_instance_id: u64,
        target_id: String,
        target_delta: PreparedTargetHostDelta,
    },
    DedicatedWorkerAttached {
        browser_context_id: String,
        renderer_instance_id: u64,
        target_id: String,
        session_id: String,
        prepared_attach: PreparedTargetAttach,
    },
    DedicatedWorkerDetached {
        target_delta: Option<PreparedTargetHostDelta>,
        cleanup_plan: TargetSessionDetachCleanupPlan,
    },
    DedicatedWorkerDestroyed {
        browser_context_id: String,
        renderer_instance_id: u64,
        target_id: String,
        target_delta: Option<PreparedTargetHostDelta>,
    },
    SharedWorkerAttachmentEvents {
        attachment: TargetSharedWorkerProtocolAttachmentIdentity,
        events: Vec<crate::conn::BackgroundProtocolEvent>,
    },
    SharedWorkerCreated {
        target_delta: PreparedTargetHostDelta,
    },
    SharedWorkerAttached {
        attachment: TargetSharedWorkerProtocolAttachmentIdentity,
        prepared_attach: PreparedTargetAttach,
    },
    ServiceWorkerVersionEvents {
        version: TargetServiceWorkerVersionIdentity,
        events: Vec<crate::conn::BackgroundProtocolEvent>,
    },
    ServiceWorkerAttachmentEvents {
        attachment: TargetServiceWorkerProtocolAttachmentIdentity,
        events: Vec<crate::conn::BackgroundProtocolEvent>,
    },
    ServiceWorkerRunEvents {
        run: TargetServiceWorkerRunIdentity,
        events: Vec<crate::conn::BackgroundProtocolEvent>,
    },
    ServiceWorkerRuntimeEvents {
        runtime: TargetServiceWorkerRuntimeAttachmentIdentity,
        events: Vec<crate::conn::BackgroundProtocolEvent>,
    },
    ServiceWorkerCreated {
        version: TargetServiceWorkerVersionIdentity,
        target_delta: PreparedTargetHostDelta,
    },
    ServiceWorkerAttached {
        attachment: TargetServiceWorkerProtocolAttachmentIdentity,
        prepared_attach: PreparedTargetAttach,
    },
    SharedWorkerDetached {
        retirement: TargetSharedWorkerProtocolAttachmentRetirement,
        cleanup_plan: TargetSessionDetachCleanupPlan,
    },
    ServiceWorkerDetached {
        retirement: TargetServiceWorkerProtocolAttachmentRetirement,
        cleanup_plan: TargetSessionDetachCleanupPlan,
    },
    SharedWorkerDestroyed {
        target_delta: PreparedTargetHostDelta,
    },
    ServiceWorkerRunRetired {
        retirement: TargetServiceWorkerRunRetirement,
    },
    ServiceWorkerDestroyed {
        retirement: TargetServiceWorkerVersionRetirement,
        target_delta: Option<PreparedTargetHostDelta>,
    },
    ServiceWorkerConsoleMessages {
        runtime: TargetServiceWorkerRuntimeAttachmentIdentity,
        messages: Vec<RuntimeConsoleMessageSnapshot>,
        console_end: usize,
    },
    SharedWorkerConsoleMessages {
        attachment: TargetSharedWorkerProtocolAttachmentIdentity,
        messages: Vec<RuntimeConsoleMessageSnapshot>,
        console_end: usize,
    },
    ServiceWorkerRuntimeConsoleMessages {
        runtime: TargetServiceWorkerRuntimeAttachmentIdentity,
        messages: Vec<RuntimeConsoleMessageSnapshot>,
        console_end: usize,
    },
    SharedWorkerRuntimeConsoleMessages {
        attachment: TargetSharedWorkerProtocolAttachmentIdentity,
        messages: Vec<RuntimeConsoleMessageSnapshot>,
        console_end: usize,
    },
    ServiceWorkerRuntimeExceptionMessages {
        runtime: TargetServiceWorkerRuntimeAttachmentIdentity,
        messages: Vec<ServiceWorkerRuntimeExceptionSnapshot>,
        exception_start: usize,
        exception_end: usize,
    },
    ServiceWorkerFetchDiagnostics {
        runtime: TargetServiceWorkerRuntimeAttachmentIdentity,
        diagnostics: Vec<RendererServiceWorkerFetchDiagnostic>,
        diagnostic_start: usize,
        diagnostic_end: usize,
    },
    ServiceWorkerRuntimeInspectorMessages {
        runtime: TargetServiceWorkerRuntimeAttachmentIdentity,
        background_events: Vec<BackgroundProtocolEvent>,
        response_events: Vec<BackgroundProtocolEvent>,
        pending_runtime_console: Option<(Vec<RuntimeConsoleMessageSnapshot>, usize)>,
        pending_runtime_exceptions:
            Option<(Vec<ServiceWorkerRuntimeExceptionSnapshot>, usize, usize)>,
    },
    SharedWorkerRuntimeInspectorMessages {
        attachment: TargetSharedWorkerProtocolAttachmentIdentity,
        messages: Vec<RendererRuntimeInspectorMessage>,
    },
    DedicatedWorkerRuntimeInspectorMessages {
        browser_context_id: String,
        renderer_instance_id: u64,
        target_id: String,
        session_id: String,
        messages: Vec<RendererRuntimeInspectorMessage>,
    },
}

#[derive(Clone, Copy)]
enum DedicatedWorkerRetirementCause {
    RendererDestroyed,
    OwnerRetired,
}

#[derive(Debug, Default, PartialEq)]
pub(in crate::domains) struct TargetPreparedOutputSlot {
    outputs: TargetPreparedOutputs,
}

impl TargetPreparedOutputs {
    fn push(&mut self, output: WorkerTargetLifecycleOutput) {
        self.worker_target_lifecycle_outputs.push(output);
    }

    pub(in crate::domains) fn extend(&mut self, other: Self) {
        self.worker_target_lifecycle_outputs
            .extend(other.worker_target_lifecycle_outputs);
    }

    pub(in crate::domains) fn is_empty(&self) -> bool {
        self.worker_target_lifecycle_outputs.is_empty()
    }

    pub(in crate::domains) fn append_to_shared_worker_target_lifecycle_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        self.append_to_target_lifecycle_output_sink_for_slots(
            sink,
            &[SLOT_SHARED_WORKER_TARGET_LIFECYCLE],
        );
    }

    pub(in crate::domains) fn append_to_service_worker_target_lifecycle_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        self.append_to_target_lifecycle_output_sink_for_slots(
            sink,
            &[SLOT_SERVICE_WORKER_TARGET_LIFECYCLE],
        );
    }

    pub(in crate::domains) fn append_to_dedicated_worker_target_lifecycle_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        self.append_to_target_lifecycle_output_sink_for_slots(
            sink,
            &[SLOT_DEDICATED_WORKER_TARGET_LIFECYCLE],
        );
    }

    pub(in crate::domains) fn append_to_target_lifecycle_output_sink_for_slots(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
        slots: &[ProtocolOutputSlot],
    ) {
        if !self.is_empty() {
            for slot in slots {
                sink.push_produced_slot(*slot);
            }
            sink.push_prepared_payload(TargetPreparedOutputSlot::from_outputs(self).into());
        }
    }
}

fn push_service_worker_version_events(
    outputs: &mut TargetPreparedOutputs,
    version: TargetServiceWorkerVersionIdentity,
    events: Vec<BackgroundProtocolEvent>,
) {
    if events.is_empty() {
        return;
    }
    outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerVersionEvents { version, events });
}

fn push_service_worker_run_events(
    outputs: &mut TargetPreparedOutputs,
    run: TargetServiceWorkerRunIdentity,
    events: Vec<BackgroundProtocolEvent>,
) {
    if events.is_empty() {
        return;
    }
    outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerRunEvents { run, events });
}

fn push_service_worker_attachment_events(
    outputs: &mut TargetPreparedOutputs,
    attachment: TargetServiceWorkerProtocolAttachmentIdentity,
    events: Vec<BackgroundProtocolEvent>,
) {
    if events.is_empty() {
        return;
    }
    outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerAttachmentEvents { attachment, events });
}

fn push_service_worker_runtime_events(
    outputs: &mut TargetPreparedOutputs,
    runtime: TargetServiceWorkerRuntimeAttachmentIdentity,
    events: Vec<BackgroundProtocolEvent>,
) {
    if events.is_empty() {
        return;
    }
    outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerRuntimeEvents { runtime, events });
}

impl TargetPreparedOutputSlot {
    pub(in crate::domains) fn from_outputs(outputs: TargetPreparedOutputs) -> Self {
        Self { outputs }
    }

    pub(in crate::domains) fn extend(&mut self, other: Self) {
        self.outputs
            .worker_target_lifecycle_outputs
            .extend(other.outputs.worker_target_lifecycle_outputs);
    }

    fn take_worker_target_lifecycle_outputs(&mut self) -> Option<Vec<WorkerTargetLifecycleOutput>> {
        (!self.outputs.worker_target_lifecycle_outputs.is_empty())
            .then(|| std::mem::take(&mut self.outputs.worker_target_lifecycle_outputs))
    }
}

pub(in crate::domains) const SLOT_SHARED_WORKER_TARGET_LIFECYCLE: ProtocolOutputSlot =
    ProtocolOutputSlot::SharedWorkerTargetLifecycle;
pub(in crate::domains) const SLOT_SERVICE_WORKER_TARGET_LIFECYCLE: ProtocolOutputSlot =
    ProtocolOutputSlot::ServiceWorkerTargetLifecycle;
pub(in crate::domains) const SLOT_DEDICATED_WORKER_TARGET_LIFECYCLE: ProtocolOutputSlot =
    ProtocolOutputSlot::DedicatedWorkerTargetLifecycle;

fn shared_worker_target_lifecycle_outputs_for_events(
    conn: &mut CdpConnection,
    browser_context_id: String,
    events: Vec<RendererSharedWorkerTargetEvent>,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    for event in events {
        match event {
            RendererSharedWorkerTargetEvent::Created(info) => {
                let owner_target_id =
                    conn.browser_context_by_id(&browser_context_id)
                        .and_then(|context| {
                            context.target_id_for_renderer_owner_local_host_id(
                                info.owner_local_host_id,
                            )
                        });
                outputs.extend(register_shared_worker_target(
                    conn,
                    &browser_context_id,
                    owner_target_id,
                    info,
                ));
            }
            RendererSharedWorkerTargetEvent::Destroyed { instance_id } => {
                outputs.extend(remove_shared_worker_target(
                    conn,
                    &browser_context_id,
                    instance_id,
                ));
            }
            RendererSharedWorkerTargetEvent::Console {
                instance_id,
                message,
            } => {
                outputs.extend(record_shared_worker_target_console_message(
                    conn,
                    &browser_context_id,
                    instance_id,
                    message,
                ));
            }
            RendererSharedWorkerTargetEvent::RuntimeInspectorMessages {
                instance_id,
                inspector_session_id,
                messages,
            } => {
                outputs.extend(record_shared_worker_target_runtime_inspector_messages(
                    conn,
                    &browser_context_id,
                    instance_id,
                    inspector_session_id,
                    messages,
                ));
            }
        }
    }
    outputs
}

pub(in crate::domains) fn shared_worker_target_lifecycle_prepared_outputs_for_event(
    conn: &mut CdpConnection,
    browser_context_id: String,
    event: RendererSharedWorkerTargetEvent,
) -> TargetPreparedOutputs {
    shared_worker_target_lifecycle_outputs_for_events(conn, browser_context_id, vec![event])
}

pub(in crate::domains) fn service_worker_target_lifecycle_prepared_outputs_for_event(
    conn: &mut CdpConnection,
    browser_context_id: String,
    event: RendererServiceWorkerTargetEvent,
) -> TargetPreparedOutputs {
    service_worker_target_lifecycle_outputs_for_events(conn, browser_context_id, vec![event])
}

pub(in crate::domains) fn dedicated_worker_target_lifecycle_prepared_outputs_for_event(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    event: RendererDedicatedWorkerTargetEvent,
) -> TargetPreparedOutputs {
    let Some(owner_page) = conn.target_page_residence_identity_for_session(session_id) else {
        return TargetPreparedOutputs::default();
    };
    let Some(owner_renderer_page) =
        conn.renderer_page_residence_identity_for_session_owner(session_id)
    else {
        return TargetPreparedOutputs::default();
    };
    let browser_context_id = owner_page.browser_context_id().to_owned();
    let owner_page_network_sessions = conn.network_event_session_ids_for_session_owner(session_id);
    dedicated_worker_target_lifecycle_outputs_for_events(
        conn,
        browser_context_id,
        owner_page,
        owner_renderer_page,
        owner_page_network_sessions,
        vec![event],
    )
}

fn dedicated_worker_target_lifecycle_outputs_for_events(
    conn: &mut CdpConnection,
    browser_context_id: String,
    owner_page: TargetPageResidenceIdentity,
    owner_renderer_page: RendererPageResidenceIdentity,
    owner_page_network_sessions: Vec<Option<String>>,
    events: Vec<RendererDedicatedWorkerTargetEvent>,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    for event in events {
        match event {
            RendererDedicatedWorkerTargetEvent::Created(info) => {
                outputs.extend(register_dedicated_worker_target(
                    conn,
                    &browser_context_id,
                    owner_page.clone(),
                    owner_renderer_page,
                    owner_page_network_sessions.clone(),
                    info,
                ));
            }
            RendererDedicatedWorkerTargetEvent::ScriptLoaded {
                instance_id,
                script_url,
                response,
            } => {
                outputs.extend(record_dedicated_worker_main_script(
                    conn,
                    &browser_context_id,
                    instance_id,
                    script_url,
                    crate::conn::DedicatedWorkerMainScriptOutcome::Loaded(response),
                ));
            }
            RendererDedicatedWorkerTargetEvent::ScriptLoadFailed {
                instance_id,
                script_url,
                error_message,
                response,
            } => {
                outputs.extend(record_dedicated_worker_main_script(
                    conn,
                    &browser_context_id,
                    instance_id,
                    script_url,
                    crate::conn::DedicatedWorkerMainScriptOutcome::Failed {
                        error_message,
                        response,
                    },
                ));
            }
            RendererDedicatedWorkerTargetEvent::Console {
                instance_id,
                message,
            } => {
                outputs.extend(record_dedicated_worker_target_console_message(
                    conn,
                    &browser_context_id,
                    instance_id,
                    message,
                ));
            }
            RendererDedicatedWorkerTargetEvent::RuntimeInspectorMessages {
                instance_id,
                inspector_session_id,
                messages,
            } => {
                outputs.extend(record_dedicated_worker_target_runtime_inspector_messages(
                    conn,
                    &browser_context_id,
                    instance_id,
                    inspector_session_id,
                    messages,
                ));
            }
            RendererDedicatedWorkerTargetEvent::Destroyed { instance_id } => {
                outputs.extend(prepare_dedicated_worker_target_retirement(
                    conn,
                    &browser_context_id,
                    instance_id,
                    DedicatedWorkerRetirementCause::RendererDestroyed,
                ));
            }
        }
    }
    outputs
}

fn service_worker_target_lifecycle_outputs_for_events(
    conn: &mut CdpConnection,
    browser_context_id: String,
    events: Vec<RendererServiceWorkerTargetEvent>,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    for event in events {
        match event {
            RendererServiceWorkerTargetEvent::Created { info, active_run } => {
                outputs.extend(register_service_worker_target_with_active_run(
                    conn,
                    &browser_context_id,
                    info,
                    active_run,
                ));
            }
            RendererServiceWorkerTargetEvent::Started { version_id, run } => {
                outputs.extend(record_service_worker_target_started(
                    conn,
                    &browser_context_id,
                    version_id,
                    run,
                ));
            }
            RendererServiceWorkerTargetEvent::Stopped {
                version_id,
                run,
                reason,
            } => {
                outputs.extend(record_service_worker_target_stopped(
                    conn,
                    &browser_context_id,
                    version_id,
                    run,
                    reason,
                ));
            }
            RendererServiceWorkerTargetEvent::Destroyed {
                version_id,
                active_run,
            } => {
                outputs.extend(remove_service_worker_target(
                    conn,
                    &browser_context_id,
                    version_id,
                    active_run,
                ));
            }
            RendererServiceWorkerTargetEvent::VersionUpdated { version_id, status } => {
                outputs.extend(record_service_worker_target_version_updated(
                    conn,
                    &browser_context_id,
                    version_id,
                    status,
                ));
            }
            RendererServiceWorkerTargetEvent::Console {
                version_id,
                run,
                message,
            } => {
                outputs.extend(record_service_worker_target_console_message(
                    conn,
                    &browser_context_id,
                    version_id,
                    run,
                    message,
                ));
            }
            RendererServiceWorkerTargetEvent::Exception {
                version_id,
                run,
                message,
            } => {
                outputs.extend(record_service_worker_target_exception_message(
                    conn,
                    &browser_context_id,
                    version_id,
                    run,
                    message,
                ));
            }
            RendererServiceWorkerTargetEvent::FetchDiagnostic {
                version_id,
                run,
                diagnostic,
            } => {
                outputs.extend(record_service_worker_target_fetch_diagnostic(
                    conn,
                    &browser_context_id,
                    version_id,
                    run,
                    diagnostic,
                ));
            }
            RendererServiceWorkerTargetEvent::RuntimeInspectorMessages {
                version_id,
                run,
                inspector_session_id,
                messages,
            } => {
                outputs.extend(record_service_worker_target_runtime_inspector_messages(
                    conn,
                    &browser_context_id,
                    version_id,
                    run,
                    inspector_session_id,
                    messages,
                ));
            }
        }
    }
    outputs
}

fn append_service_worker_domain_snapshot(
    conn: &CdpConnection,
    browser_context_id: &str,
    version: TargetServiceWorkerVersionIdentity,
    outputs: &mut TargetPreparedOutputs,
) {
    let session_ids =
        service_worker::enabled_sessions_for_browser_context(conn, browser_context_id);
    if session_ids.is_empty() {
        return;
    }
    let events =
        service_worker::snapshot_events_for_browser_context(conn, browser_context_id, &session_ids);
    push_service_worker_version_events(outputs, version, events);
}

fn dedicated_worker_target_is_current(
    conn: &CdpConnection,
    browser_context_id: &str,
    renderer_instance_id: u64,
    target_id: &str,
) -> bool {
    let Some(context) = conn.browser_context_by_id(browser_context_id) else {
        return false;
    };
    context
        .dedicated_worker_targets
        .get(&renderer_instance_id)
        .is_some_and(|target| {
            target.target_id == target_id
                && context.target_page_residence_is_current(&target.owner_page)
        })
}

fn register_dedicated_worker_target(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    owner_page: TargetPageResidenceIdentity,
    owner_renderer_page: RendererPageResidenceIdentity,
    owner_page_network_sessions: Vec<Option<String>>,
    info: RendererDedicatedWorkerTargetInfo,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    if info.owner_local_host_id != owner_renderer_page.owner_local_host_id()
        || info.page_id != owner_renderer_page.page_id()
    {
        return outputs;
    }
    if conn
        .browser_context_by_id(browser_context_id)
        .and_then(|context| {
            context.dedicated_worker_target_id_for_renderer_instance(info.instance_id)
        })
        .is_some()
    {
        return outputs;
    }
    let target_id = conn.gen_target_id();
    let request_url = match Url::parse(&info.request_url) {
        Ok(url) => url,
        Err(_) => return outputs,
    };
    let document_url = match Url::parse(&info.document_url) {
        Ok(url) => url,
        Err(_) => return outputs,
    };
    let owner_target_id = owner_page.target_id().unwrap_or_default().to_owned();
    let should_emit_created = conn.has_any_target_discovery();
    let created_snapshot = {
        let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
            return TargetPreparedOutputs::default();
        };
        context.insert_dedicated_worker_target(crate::conn::DedicatedWorkerTargetState::new(
            owner_page.clone(),
            info.owner_local_host_id,
            info.instance_id,
            target_id.clone(),
            info.name,
            owner_page_network_sessions.clone(),
        ));
        should_emit_created
            .then(|| context.devtools_target_info(&target_id))
            .flatten()
    };
    conn.register_worker_target_host(&target_id, DevToolsTargetKind::Worker);
    if let Some(target_info) = created_snapshot {
        outputs.push(WorkerTargetLifecycleOutput::DedicatedWorkerCreated {
            browser_context_id: browser_context_id.to_owned(),
            renderer_instance_id: info.instance_id,
            target_delta: PreparedTargetHostDelta::created(target_id.clone(), Some(target_info)),
        });
    }
    let timestamp = monotonic_timestamp_seconds();
    for session_id in &owner_page_network_sessions {
        let mut events = Vec::new();
        network::emit_request_will_be_sent(
            &mut events,
            session_id.as_deref(),
            &target_id,
            &owner_target_id,
            &owner_target_id,
            timestamp,
            &document_url,
            &request_url,
            "GET",
            None,
            &[],
            DevToolsNetworkResourceType::Script,
            SubresourceRequestInitiatorType::Other,
            None,
            false,
            None,
            &[],
        );
        if !events.is_empty() {
            outputs.push(WorkerTargetLifecycleOutput::DedicatedWorkerEvents {
                browser_context_id: browser_context_id.to_owned(),
                renderer_instance_id: info.instance_id,
                target_id: target_id.clone(),
                events,
            });
        }
    }
    outputs
}

fn record_dedicated_worker_main_script(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_instance_id: u64,
    script_url: String,
    outcome: crate::conn::DedicatedWorkerMainScriptOutcome,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let Some(owner_page) = conn
        .browser_context_by_id(browser_context_id)
        .and_then(|context| context.dedicated_worker_targets.get(&renderer_instance_id))
        .map(|target| target.owner_page.clone())
    else {
        return outputs;
    };
    let auto_attach_owners = dedicated_worker_auto_attach_owner_sessions(conn, &owner_page);
    let attached_sessions = auto_attach_owners
        .into_iter()
        .map(|owner| {
            let waiting = conn.auto_attach_owner_waits_for_debugger_on_start(owner.as_deref());
            (owner, conn.gen_session_id(), waiting)
        })
        .collect::<Vec<_>>();
    let pause_failed_target_until_debugger_resume = attached_sessions
        .iter()
        .any(|(_, _, waiting_for_debugger)| *waiting_for_debugger);
    let should_emit_info_changed = conn.has_any_target_discovery();
    let (target_id, page_extra_events) = {
        let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
            return outputs;
        };
        let Some(target) = context
            .dedicated_worker_targets
            .get_mut(&renderer_instance_id)
        else {
            return outputs;
        };
        let target_id = target.target_id.clone();
        let owner_page_network_sessions = target.owner_page_network_sessions.clone();
        let page_extra_events = dedicated_worker_main_script_page_extra_events(
            &target_id,
            &owner_page_network_sessions,
            &outcome,
        );
        target.record_main_script(
            script_url,
            outcome,
            pause_failed_target_until_debugger_resume,
        );
        (target_id, page_extra_events)
    };
    for (_session_id, events) in page_extra_events {
        outputs.push(WorkerTargetLifecycleOutput::DedicatedWorkerEvents {
            browser_context_id: browser_context_id.to_owned(),
            renderer_instance_id,
            target_id: target_id.clone(),
            events,
        });
    }
    let mut prepared_attaches = Vec::new();
    for (owner_session_id, session_id, waiting_for_debugger) in attached_sessions {
        let Some(target_info) = conn
            .prepare_auto_attached_dedicated_worker_session_binding_info_in_browser_context(
                browser_context_id,
                &target_id,
                session_id.clone(),
            )
        else {
            continue;
        };
        let prepared_session = conn.prepare_auto_attach_session_commit(
            session_id.clone(),
            owner_session_id,
            waiting_for_debugger,
        );
        if waiting_for_debugger
            && let Some(context) = conn.browser_context_by_id_mut(browser_context_id)
            && let Some(target) = context
                .dedicated_worker_targets
                .get_mut(&renderer_instance_id)
        {
            target.allow_main_script_network_replay_to(&session_id);
        }
        prepared_attaches.push(WorkerTargetLifecycleOutput::DedicatedWorkerAttached {
            browser_context_id: browser_context_id.to_owned(),
            renderer_instance_id,
            target_id: target_id.clone(),
            session_id,
            prepared_attach: PreparedTargetAttach::new(
                target_id.clone(),
                target_info,
                [prepared_session],
            ),
        });
    }
    let changed_snapshot = should_emit_info_changed
        .then(|| {
            conn.browser_context_by_id(browser_context_id)
                .and_then(|context| context.devtools_target_info(&target_id))
        })
        .flatten();
    if let Some(target_info) = changed_snapshot {
        outputs.push(WorkerTargetLifecycleOutput::DedicatedWorkerInfoChanged {
            browser_context_id: browser_context_id.to_owned(),
            renderer_instance_id,
            target_id: target_id.clone(),
            target_delta: PreparedTargetHostDelta::info_changed(
                target_id.clone(),
                Some(target_info),
            ),
        });
    }
    outputs
        .worker_target_lifecycle_outputs
        .extend(prepared_attaches);
    let enabled_sessions = conn
        .browser_context_by_id(browser_context_id)
        .and_then(|context| context.dedicated_worker_targets.get(&renderer_instance_id))
        .map(|target| {
            target
                .session_ids()
                .into_iter()
                .filter(|session_id| {
                    target.network_enabled(session_id)
                        && !target.main_script_was_delivered_to(session_id)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for session_id in enabled_sessions {
        let events = conn
            .browser_context_by_id(browser_context_id)
            .and_then(|context| context.dedicated_worker_targets.get(&renderer_instance_id))
            .and_then(|target| target.main_script())
            .map(|script| {
                dedicated_worker_main_script_worker_events(&target_id, Some(&session_id), script)
            })
            .unwrap_or_default();
        if let Some(context) = conn.browser_context_by_id_mut(browser_context_id)
            && let Some(target) = context
                .dedicated_worker_targets
                .get_mut(&renderer_instance_id)
        {
            target.mark_main_script_delivered_to(&session_id);
        }
        if !events.is_empty() {
            outputs.push(WorkerTargetLifecycleOutput::DedicatedWorkerEvents {
                browser_context_id: browser_context_id.to_owned(),
                renderer_instance_id,
                target_id: target_id.clone(),
                events,
            });
        }
    }
    outputs
}

pub(super) fn dedicated_worker_auto_attach_owner_session_allowed(
    conn: &CdpConnection,
    owner_session_id: Option<&str>,
    owner_page: &TargetPageResidenceIdentity,
) -> bool {
    let Some(owner_session_id) = owner_session_id else {
        return false;
    };
    conn.target_page_residence_identity_for_session(Some(owner_session_id))
        .as_ref()
        == Some(owner_page)
}

fn shared_worker_auto_attach_owner_sessions(conn: &CdpConnection) -> Vec<Option<String>> {
    conn.auto_attach_owner_sessions_for_target_type("shared_worker")
        .into_iter()
        .filter(|owner_session_id| {
            super::browser_level_auto_attach_owner_session_allowed(
                conn,
                owner_session_id.as_deref(),
            )
        })
        .collect()
}

fn dedicated_worker_auto_attach_owner_sessions(
    conn: &CdpConnection,
    owner_page: &TargetPageResidenceIdentity,
) -> Vec<Option<String>> {
    conn.auto_attach_owner_sessions_for_target_type("worker")
        .into_iter()
        .filter(|owner_session_id| {
            dedicated_worker_auto_attach_owner_session_allowed(
                conn,
                owner_session_id.as_deref(),
                owner_page,
            )
        })
        .collect()
}

fn dedicated_worker_main_script_page_extra_events(
    request_id: &str,
    sessions: &[Option<String>],
    outcome: &crate::conn::DedicatedWorkerMainScriptOutcome,
) -> Vec<(Option<String>, Vec<BackgroundProtocolEvent>)> {
    let response = match outcome {
        crate::conn::DedicatedWorkerMainScriptOutcome::Loaded(response) => Some(response.as_ref()),
        crate::conn::DedicatedWorkerMainScriptOutcome::Failed { response, .. } => {
            response.as_deref()
        }
    };
    let Some(response) = response else {
        return Vec::new();
    };
    let has_network_extra_info =
        response.network_request_headers().is_some() || response.request_cookie_report.is_some();
    if !has_network_extra_info {
        return Vec::new();
    }
    let default_cookie_report = moli_cookie_jar::StoredCookieQueryReport::default();
    let cookie_report = response
        .request_cookie_report
        .as_ref()
        .unwrap_or(&default_cookie_report);
    sessions
        .iter()
        .map(|session_id| {
            let mut events = Vec::new();
            network::emit_request_will_be_sent_extra_info(
                &mut events,
                session_id.as_deref(),
                request_id,
                response.network_request_headers().unwrap_or_default(),
                cookie_report,
                monotonic_timestamp_seconds(),
            );
            network::emit_response_received_extra_info(
                &mut events,
                session_id.as_deref(),
                request_id,
                &response.headers,
                response.status,
                &response.cookie_set_reports,
            );
            (session_id.clone(), events)
        })
        .collect()
}

fn dedicated_worker_main_script_worker_events(
    target_id: &str,
    session_id: Option<&str>,
    script: &crate::conn::DedicatedWorkerMainScriptSnapshot,
) -> Vec<BackgroundProtocolEvent> {
    let mut events = Vec::new();
    let timestamp = monotonic_timestamp_seconds();
    let response = match &script.outcome {
        crate::conn::DedicatedWorkerMainScriptOutcome::Loaded(response) => Some(response.as_ref()),
        crate::conn::DedicatedWorkerMainScriptOutcome::Failed { response, .. } => {
            response.as_deref()
        }
    };
    if let Some(response) = response {
        let has_extra_info = response.network_request_headers().is_some()
            || response.request_cookie_report.is_some();
        network::emit_response_received_without_extra_info_event(
            &mut events,
            session_id,
            target_id,
            target_id,
            target_id,
            timestamp,
            &response.final_url,
            response.status,
            None,
            &response.headers,
            response.body_bytes().len(),
            response.from_cache,
            response.negotiated_http_version,
            has_extra_info,
            DevToolsNetworkResourceType::Script,
        );
    }
    match &script.outcome {
        crate::conn::DedicatedWorkerMainScriptOutcome::Loaded(response) => {
            network::emit_loading_finished(
                &mut events,
                session_id,
                target_id,
                target_id,
                target_id,
                timestamp,
                response.body_bytes().len(),
                DevToolsNetworkResourceType::Script,
            );
        }
        crate::conn::DedicatedWorkerMainScriptOutcome::Failed { error_message, .. } => {
            network::emit_loading_failed(
                &mut events,
                session_id,
                target_id,
                target_id,
                target_id,
                timestamp,
                dedicated_worker_loading_error_text(error_message),
                DevToolsNetworkResourceType::Script,
            );
        }
    }
    events
}

fn dedicated_worker_loading_error_text(error_message: &str) -> &str {
    error_message
        .split_ascii_whitespace()
        .find(|part| part.starts_with("net::ERR_"))
        .map(|part| {
            part.trim_end_matches(|character: char| {
                !character.is_ascii_alphanumeric() && character != '_'
            })
        })
        .unwrap_or("net::ERR_FAILED")
}

pub(in crate::domains) fn dedicated_worker_main_script_network_replay_for_session(
    conn: &mut CdpConnection,
    session_id: &str,
) -> Vec<BackgroundProtocolEvent> {
    let Some(crate::conn::CdpSessionRoute::DedicatedWorkerTarget {
        browser_context_id,
        target_id,
    }) = conn.session_route(Some(session_id))
    else {
        return Vec::new();
    };
    let Some(renderer_instance_id) = conn
        .browser_context_by_id(&browser_context_id)
        .and_then(|context| context.dedicated_worker_target(&target_id))
        .map(|target| target.renderer_instance_id)
    else {
        return Vec::new();
    };
    let events = conn
        .browser_context_by_id(&browser_context_id)
        .and_then(|context| context.dedicated_worker_targets.get(&renderer_instance_id))
        .filter(|target| {
            target.network_enabled(session_id)
                && target.main_script_network_replay_allowed_for(session_id)
                && !target.main_script_was_delivered_to(session_id)
        })
        .and_then(|target| target.main_script())
        .map(|script| {
            dedicated_worker_main_script_worker_events(&target_id, Some(session_id), script)
        })
        .unwrap_or_default();
    if events.is_empty() {
        return events;
    }
    if let Some(context) = conn.browser_context_by_id_mut(&browser_context_id)
        && let Some(target) = context
            .dedicated_worker_targets
            .get_mut(&renderer_instance_id)
    {
        target.mark_main_script_delivered_to(session_id);
    }
    events
}

pub(in crate::domains) fn release_failed_dedicated_worker_target_after_debugger_resume(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) -> Option<Vec<BackgroundProtocolEvent>> {
    let session_id = session_id?;
    let crate::conn::CdpSessionRoute::DedicatedWorkerTarget {
        browser_context_id,
        target_id,
    } = conn.session_route(Some(session_id))?
    else {
        return None;
    };
    let renderer_instance_id = {
        let target = conn
            .browser_context_by_id_mut(&browser_context_id)?
            .dedicated_worker_target_mut(&target_id)?;
        if !target.release_deferred_renderer_destroyed_for_debugger_resume() {
            return None;
        }
        target.renderer_instance_id
    };
    let outputs = prepare_dedicated_worker_target_retirement(
        conn,
        &browser_context_id,
        renderer_instance_id,
        DedicatedWorkerRetirementCause::OwnerRetired,
    );
    Some(commit_failed_dedicated_worker_retirement_sync(
        conn, outputs,
    ))
}

pub(in crate::domains) async fn retire_dedicated_worker_targets_for_replaced_page_async(
    conn: &mut CdpConnection,
    replaced_page_owner: &TargetPageResidenceIdentity,
) -> Vec<BackgroundProtocolEvent> {
    let renderer_instance_ids = conn
        .browser_context_by_id(replaced_page_owner.browser_context_id())
        .map(|context| {
            context
                .dedicated_worker_targets
                .iter()
                .filter_map(|(renderer_instance_id, target)| {
                    (&target.owner_page == replaced_page_owner).then_some(*renderer_instance_id)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut events = Vec::new();
    for renderer_instance_id in renderer_instance_ids {
        let outputs = prepare_dedicated_worker_target_retirement(
            conn,
            replaced_page_owner.browser_context_id(),
            renderer_instance_id,
            DedicatedWorkerRetirementCause::OwnerRetired,
        );
        for output in outputs.worker_target_lifecycle_outputs {
            match commit_dedicated_worker_retirement_output_async(conn, output).await {
                Ok(output_events) => events.extend(output_events),
                Err(output) => {
                    debug_assert!(
                        false,
                        "Page replacement DedicatedWorker retirement contained non-terminal output: {output:?}"
                    );
                }
            }
        }
    }
    events
}

async fn commit_dedicated_worker_retirement_output_async(
    conn: &mut CdpConnection,
    output: WorkerTargetLifecycleOutput,
) -> Result<Vec<BackgroundProtocolEvent>, WorkerTargetLifecycleOutput> {
    let mut events = Vec::new();
    match output {
        WorkerTargetLifecycleOutput::DedicatedWorkerDetached {
            target_delta,
            cleanup_plan,
        } => {
            if let Some(target_delta) = target_delta {
                events.extend(
                    conn.prepared_target_info_changed_event_plan_for_discovery_owners(target_delta),
                );
            }
            events.extend(
                conn.detach_dedicated_worker_session_with_binding_cleanup_event_plan_async(
                    cleanup_plan,
                )
                .await,
            );
        }
        WorkerTargetLifecycleOutput::DedicatedWorkerDestroyed {
            browser_context_id,
            renderer_instance_id,
            target_id,
            target_delta,
        } => {
            let removed = conn
                .browser_context_by_id_mut(&browser_context_id)
                .and_then(|context| {
                    let target = context
                        .dedicated_worker_targets
                        .get(&renderer_instance_id)?;
                    (target.target_id == target_id).then(|| {
                        context.remove_dedicated_worker_target_by_renderer_instance(
                            renderer_instance_id,
                        )
                    })
                })
                .flatten();
            if removed.is_none() {
                return Ok(events);
            }
            conn.remove_worker_target_host(&target_id);
            if let Some(target_delta) = target_delta {
                events.extend(conn.prepared_target_host_delta_event_plan(target_delta));
            }
        }
        output => return Err(output),
    }
    Ok(events)
}

fn commit_failed_dedicated_worker_retirement_sync(
    conn: &mut CdpConnection,
    outputs: TargetPreparedOutputs,
) -> Vec<BackgroundProtocolEvent> {
    let retirement_identity = outputs
        .worker_target_lifecycle_outputs
        .iter()
        .find_map(|output| match output {
            WorkerTargetLifecycleOutput::DedicatedWorkerDestroyed {
                browser_context_id,
                renderer_instance_id,
                target_id,
                ..
            } => Some((
                browser_context_id.clone(),
                *renderer_instance_id,
                target_id.clone(),
            )),
            _ => None,
        });
    let Some((browser_context_id, renderer_instance_id, target_id)) = retirement_identity else {
        return Vec::new();
    };
    let removed = conn
        .browser_context_by_id_mut(&browser_context_id)
        .and_then(|context| {
            let target = context
                .dedicated_worker_targets
                .get(&renderer_instance_id)?;
            (target.target_id == target_id).then(|| {
                context.remove_dedicated_worker_target_by_renderer_instance(renderer_instance_id)
            })
        })
        .flatten();
    if removed.is_none() {
        return Vec::new();
    }
    conn.remove_worker_target_host(&target_id);

    let mut events = Vec::new();
    for output in outputs.worker_target_lifecycle_outputs {
        match output {
            WorkerTargetLifecycleOutput::DedicatedWorkerDetached {
                target_delta,
                cleanup_plan,
            } => {
                if let Some(target_delta) = target_delta {
                    events.extend(
                        conn.prepared_target_info_changed_event_plan_for_discovery_owners(
                            target_delta,
                        ),
                    );
                }
                events.extend(
                    conn.detach_dedicated_worker_session_after_target_removal_event_plan(
                        cleanup_plan,
                    ),
                );
            }
            WorkerTargetLifecycleOutput::DedicatedWorkerDestroyed { target_delta, .. } => {
                if let Some(target_delta) = target_delta {
                    events.extend(conn.prepared_target_host_delta_event_plan(target_delta));
                }
            }
            output => {
                debug_assert!(
                    false,
                    "failed DedicatedWorker retirement contained non-terminal output: {output:?}"
                );
            }
        }
    }
    events
}

fn record_dedicated_worker_target_runtime_inspector_messages(
    conn: &CdpConnection,
    browser_context_id: &str,
    renderer_instance_id: u64,
    inspector_session_id: Option<String>,
    messages: Vec<RendererRuntimeInspectorMessage>,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let Some(target) = conn
        .browser_context_by_id(browser_context_id)
        .and_then(|context| context.dedicated_worker_targets.get(&renderer_instance_id))
    else {
        return outputs;
    };
    let target_id = target.target_id.clone();
    let session_ids = if let Some(session_id) = inspector_session_id {
        target
            .is_session(&session_id)
            .then_some(vec![session_id])
            .unwrap_or_default()
    } else {
        target.session_ids()
    };
    for session_id in session_ids {
        outputs.push(
            WorkerTargetLifecycleOutput::DedicatedWorkerRuntimeInspectorMessages {
                browser_context_id: browser_context_id.to_owned(),
                renderer_instance_id,
                target_id: target_id.clone(),
                session_id,
                messages: messages.clone(),
            },
        );
    }
    outputs
}

fn record_dedicated_worker_target_console_message(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_instance_id: u64,
    message: RendererSharedWorkerConsoleMessage,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
        return outputs;
    };
    let Some(target) = context
        .dedicated_worker_targets
        .get_mut(&renderer_instance_id)
    else {
        return outputs;
    };
    target.record_console_message(message);
    let target_id = target.target_id.clone();
    let console_end = target.console_message_count();
    for session_id in target.session_ids() {
        let console_messages = target.pending_console_domain_messages(&session_id).to_vec();
        let runtime_messages = target
            .pending_runtime_console_messages(&session_id)
            .to_vec();
        if console_messages.is_empty() && runtime_messages.is_empty() {
            continue;
        }
        outputs.push(
            WorkerTargetLifecycleOutput::DedicatedWorkerConsoleMessages {
                browser_context_id: browser_context_id.to_owned(),
                renderer_instance_id,
                target_id: target_id.clone(),
                session_id,
                console_messages,
                runtime_messages,
                console_end,
            },
        );
    }
    outputs
}

fn prepare_dedicated_worker_target_retirement(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_instance_id: u64,
    cause: DedicatedWorkerRetirementCause,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    if matches!(cause, DedicatedWorkerRetirementCause::RendererDestroyed)
        && conn
            .browser_context_by_id_mut(browser_context_id)
            .and_then(|context| {
                context
                    .dedicated_worker_targets
                    .get_mut(&renderer_instance_id)
            })
            .is_some_and(|target| target.defer_renderer_destroyed_for_debugger_resume())
    {
        return outputs;
    }
    let target_id = match conn
        .browser_context_by_id(browser_context_id)
        .and_then(|context| {
            context.dedicated_worker_target_id_for_renderer_instance(renderer_instance_id)
        }) {
        Some(target_id) => target_id.to_owned(),
        None => return outputs,
    };
    let destroyed_delta = conn
        .has_any_target_discovery()
        .then(|| conn.prepare_destroyed_target_host_delta(&target_id))
        .flatten();
    let mut detached_delta = conn
        .browser_context_by_id(browser_context_id)
        .and_then(|context| {
            let target = context.dedicated_worker_target(&target_id)?;
            if !target.has_session() {
                return None;
            }
            let mut target_info = context.devtools_target_info(&target_id)?;
            target_info.attached = false;
            Some(PreparedTargetHostDelta::info_changed(
                target_id.clone(),
                Some(target_info),
            ))
        });
    let Some(context) = conn.browser_context_by_id(browser_context_id) else {
        return outputs;
    };
    let Some(target) = context.dedicated_worker_targets.get(&renderer_instance_id) else {
        return outputs;
    };
    for session_id in target.session_ids() {
        outputs.push(WorkerTargetLifecycleOutput::DedicatedWorkerDetached {
            target_delta: detached_delta.take(),
            cleanup_plan: TargetSessionDetachCleanupPlan::new(
                target_id.clone(),
                session_id,
                None,
                None,
            ),
        });
    }
    outputs.push(WorkerTargetLifecycleOutput::DedicatedWorkerDestroyed {
        browser_context_id: browser_context_id.to_owned(),
        renderer_instance_id,
        target_id,
        target_delta: destroyed_delta,
    });
    outputs
}

fn register_shared_worker_target(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    owner_target_id: Option<String>,
    info: RendererSharedWorkerTargetInfo,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    if conn
        .browser_context_by_id(browser_context_id)
        .and_then(|context| context.shared_worker_target_id_for_renderer_instance(info.instance_id))
        .is_some()
    {
        return outputs;
    }
    let target_id = conn.gen_target_id();
    let should_emit_created = conn.has_any_target_discovery();
    let auto_attach_owners = shared_worker_auto_attach_owner_sessions(conn);
    let attached_sessions = auto_attach_owners
        .iter()
        .map(|owner| {
            (
                owner.clone(),
                conn.gen_session_id(),
                conn.auto_attach_owner_waits_for_debugger_on_start(owner.as_deref()),
            )
        })
        .collect::<Vec<_>>();
    let created_snapshot = {
        let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
            return outputs;
        };
        context.insert_shared_worker_target(SharedWorkerTargetState::new(
            info.owner_local_host_id,
            info.instance_id,
            target_id.clone(),
            owner_target_id,
            info.url,
            info.name,
        ));
        if should_emit_created {
            let snapshot = context.devtools_target_info(&target_id);
            debug_assert!(snapshot.is_some());
            snapshot
        } else {
            None
        }
    };
    let mut attached_outputs = Vec::new();
    for (owner_session_id, session_id, waiting_for_debugger) in attached_sessions {
        if let Some(target_info) = conn
            .prepare_auto_attached_shared_worker_session_binding_info_in_browser_context(
                browser_context_id,
                &target_id,
                session_id.clone(),
            )
        {
            let attachment = conn
                .shared_worker_protocol_attachment_identity_for_session(Some(&session_id))
                .expect("new shared-worker session must expose its exact attachment identity");
            let prepared_session = conn.prepare_auto_attach_session_commit(
                session_id,
                owner_session_id,
                waiting_for_debugger,
            );
            assert!(
                matches!(
                    prepared_session.route(),
                    Some(crate::conn::CdpSessionRoute::SharedWorkerTarget {
                        browser_context_id: route_browser_context_id,
                        target_id: route_target_id,
                    }) if route_browser_context_id == browser_context_id
                        && route_target_id == &target_id
                ),
                "shared-worker auto-attach must freeze its exact target route at capture"
            );
            attached_outputs.push((
                attachment,
                PreparedTargetAttach::new(target_id.clone(), target_info, [prepared_session]),
            ));
        }
    }
    conn.register_worker_target_host(&target_id, DevToolsTargetKind::SharedWorker);
    if let Some(target_info) = created_snapshot {
        outputs.push(WorkerTargetLifecycleOutput::SharedWorkerCreated {
            target_delta: PreparedTargetHostDelta::created(target_id.clone(), Some(target_info)),
        });
    }
    for (attachment, prepared_attach) in attached_outputs {
        outputs.push(WorkerTargetLifecycleOutput::SharedWorkerAttached {
            attachment,
            prepared_attach,
        });
    }
    outputs
}

fn register_service_worker_target_with_active_run(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    info: RendererServiceWorkerTargetInfo,
    active_renderer_run: Option<RendererServiceWorkerRunIdentity>,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    if conn
        .browser_context_by_id(browser_context_id)
        .and_then(|context| context.service_worker_target_id_for_renderer_version(info.version_id))
        .is_some()
    {
        return outputs;
    }
    let target_id = conn.gen_target_id();
    let should_emit_created = conn.has_any_target_discovery();
    let mut auto_attach_owners = conn
        .auto_attach_owner_sessions_for_target_type("service_worker")
        .into_iter()
        .map(|owner_session_id| {
            let waiting_for_debugger =
                conn.auto_attach_owner_waits_for_debugger_on_start(owner_session_id.as_deref());
            (owner_session_id, waiting_for_debugger)
        })
        .collect::<Vec<_>>();
    let related_auto_attach_owners = conn
        .service_worker_auto_attach_related_owner_sessions_for_target(
            browser_context_id,
            info.registration_id,
            info.version_id,
            &info.script_url,
            &info.scope_url,
        );
    let should_pause_on_start_for_related_devtools = related_auto_attach_owners
        .iter()
        .any(|owner| owner.wait_for_debugger_on_start);
    for owner in related_auto_attach_owners {
        if !auto_attach_owners
            .iter()
            .any(|(existing_owner, _)| *existing_owner == owner.owner_session_id)
        {
            auto_attach_owners.push((owner.owner_session_id, owner.wait_for_debugger_on_start));
        }
    }
    let attached_sessions = auto_attach_owners
        .into_iter()
        .map(|(owner, waiting_for_debugger)| (owner, conn.gen_session_id(), waiting_for_debugger))
        .collect::<Vec<_>>();
    let (created_snapshot, version) = {
        let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
            return outputs;
        };
        if should_pause_on_start_for_related_devtools {
            context
                .renderer_runtime()
                .set_service_worker_pause_on_start_for_version_for_devtools(info.version_id, true);
        }
        context.insert_service_worker_target(ServiceWorkerTargetState::new(
            info.registration_id,
            info.version_id,
            target_id.clone(),
            info.script_url,
            info.scope_url,
            info.status,
            active_renderer_run,
        ));
        let target = context
            .service_worker_target(&target_id)
            .expect("inserted service-worker target must remain resident");
        let version = target
            .version_identity(browser_context_id)
            .expect("inserted service-worker target must own its version scope");
        let snapshot = if should_emit_created {
            let snapshot = context.devtools_target_info(&target_id);
            debug_assert!(snapshot.is_some());
            snapshot
        } else {
            None
        };
        (snapshot, version)
    };
    let mut attached_outputs = Vec::new();
    for (owner_session_id, session_id, waiting_for_debugger) in attached_sessions {
        if let Some(target_info) = conn
            .prepare_auto_attached_service_worker_session_binding_info_in_browser_context(
                browser_context_id,
                &target_id,
                session_id.clone(),
            )
        {
            let prepared_session = conn.prepare_auto_attach_session_commit(
                session_id.clone(),
                owner_session_id,
                waiting_for_debugger,
            );
            let prepared_attach =
                PreparedTargetAttach::new(target_id.clone(), target_info, [prepared_session]);
            let attachment = conn
                .browser_context_by_id(browser_context_id)
                .and_then(|context| context.service_worker_target(&target_id))
                .and_then(|target| {
                    target.protocol_attachment_identity(browser_context_id, &session_id)
                })
                .expect("auto-attached service-worker session must own an exact attachment");
            attached_outputs.push((attachment, prepared_attach));
        }
    }
    conn.register_worker_target_host(&target_id, DevToolsTargetKind::ServiceWorker);
    if let Some(target_info) = created_snapshot {
        outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerCreated {
            version: version.clone(),
            target_delta: PreparedTargetHostDelta::created(target_id.clone(), Some(target_info)),
        });
    }
    for (attachment, prepared_attach) in attached_outputs {
        outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerAttached {
            attachment,
            prepared_attach,
        });
    }
    append_service_worker_domain_snapshot(conn, browser_context_id, version, &mut outputs);
    outputs
}

#[cfg(test)]
fn register_service_worker_target(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    info: RendererServiceWorkerTargetInfo,
) -> TargetPreparedOutputs {
    register_service_worker_target_with_active_run(conn, browser_context_id, info, None)
}

fn record_service_worker_target_started(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_version_id: u64,
    renderer_run: RendererServiceWorkerRunIdentity,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let Some((version, reloaded_events)) = ({
        let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
            return outputs;
        };
        let Some(target_id) = context
            .service_worker_target_id_for_renderer_version(renderer_version_id)
            .map(str::to_owned)
        else {
            return outputs;
        };
        let Some(target) = context.service_worker_target_mut(&target_id) else {
            return outputs;
        };
        let Some(run) = target.mark_worker_started(browser_context_id, renderer_run) else {
            return outputs;
        };
        let reloaded_events = target
            .take_inspector_target_reloaded_after_crash_session_ids()
            .into_iter()
            .filter_map(|session_id| {
                let runtime = target.runtime_attachment_identity_for_run(
                    browser_context_id,
                    &session_id,
                    &run,
                )?;
                Some((
                    runtime,
                    BackgroundProtocolEvent::inspector_target_reloaded_after_crash(Some(
                        &session_id,
                    )),
                ))
            })
            .collect::<Vec<_>>();
        Some((run.version().clone(), reloaded_events))
    }) else {
        return outputs;
    };
    for (runtime, event) in reloaded_events {
        push_service_worker_runtime_events(&mut outputs, runtime, vec![event]);
    }
    append_service_worker_domain_snapshot(conn, browser_context_id, version, &mut outputs);
    outputs
}

fn record_service_worker_target_stopped(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_version_id: u64,
    renderer_run: RendererServiceWorkerRunIdentity,
    reason: String,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let Some((
        target_id,
        version,
        session_runtimes,
        inspector_crashed_session_ids,
        context_reported_session_ids,
        retirement,
    )) = ({
        let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
            return outputs;
        };
        let Some(target_id) = context
            .service_worker_target_id_for_renderer_version(renderer_version_id)
            .map(str::to_owned)
        else {
            return outputs;
        };
        let Some(target) = context.service_worker_target_mut(&target_id) else {
            return outputs;
        };
        let Some(retirement) =
            target.mark_worker_stopped(browser_context_id, renderer_run, &reason)
        else {
            return outputs;
        };
        let version = retirement.identity().version().clone();
        let session_runtimes = target
            .session_ids()
            .into_iter()
            .filter_map(|session_id| {
                let attachment =
                    target.protocol_attachment_identity(browser_context_id, &session_id)?;
                Some((
                    session_id,
                    TargetServiceWorkerRuntimeAttachmentIdentity::new(
                        attachment,
                        retirement.identity().clone(),
                    ),
                ))
            })
            .collect::<Vec<_>>();
        let inspector_crashed_session_ids = target.inspector_enabled_session_ids();
        for session_id in &inspector_crashed_session_ids {
            target.record_inspector_target_crashed_for_session(session_id);
        }
        Some((
            target_id,
            version,
            session_runtimes,
            inspector_crashed_session_ids,
            target.runtime_context_reported_session_ids(),
            retirement,
        ))
    })
    else {
        return outputs;
    };

    for (session_id, runtime) in &session_runtimes {
        let mut pending_await_direct_events = Vec::new();
        let mut pending_await_claimed_events = Vec::new();
        conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
            &mut pending_await_direct_events,
            &mut pending_await_claimed_events,
            Some(session_id),
            "Service worker stopped",
        );
        pending_await_direct_events.extend(pending_await_claimed_events);
        push_service_worker_runtime_events(
            &mut outputs,
            runtime.clone(),
            pending_await_direct_events,
        );
    }

    for session_id in inspector_crashed_session_ids {
        let Some((_, runtime)) = session_runtimes
            .iter()
            .find(|(candidate, _)| *candidate == session_id)
        else {
            continue;
        };
        push_service_worker_runtime_events(
            &mut outputs,
            runtime.clone(),
            vec![BackgroundProtocolEvent::inspector_target_crashed(Some(
                &session_id,
            ))],
        );
    }

    if !context_reported_session_ids.is_empty() {
        let mut runtime_context_cleared = Vec::new();
        if let Some(context) = conn.browser_context_by_id_mut(browser_context_id)
            && let Some(target) = context.service_worker_target_mut(&target_id)
        {
            for session_id in context_reported_session_ids {
                if target.is_session(&session_id) {
                    target.clear_runtime_remote_object_tracking(&session_id);
                    target.record_runtime_contexts_cleared_for_frontend(&session_id);
                    if let Some((_, runtime)) = session_runtimes
                        .iter()
                        .find(|(candidate, _)| *candidate == session_id)
                    {
                        runtime_context_cleared.push((session_id, runtime.clone()));
                    }
                }
            }
        }
        for (session_id, runtime) in runtime_context_cleared {
            push_service_worker_runtime_events(
                &mut outputs,
                runtime,
                vec![BackgroundProtocolEvent::runtime_execution_contexts_cleared(
                    Some(&session_id),
                    RuntimeExecutionContextsClearedEvent { target_id: None },
                )],
            );
        }
    }

    append_service_worker_domain_snapshot(conn, browser_context_id, version, &mut outputs);
    outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerRunRetired { retirement });
    outputs
}

fn record_service_worker_target_version_updated(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_version_id: u64,
    status: RendererServiceWorkerVersionStatus,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let Some(version) = ({
        let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
            return outputs;
        };
        let Some(target_id) = context
            .service_worker_target_id_for_renderer_version(renderer_version_id)
            .map(str::to_owned)
        else {
            return outputs;
        };
        let Some(target) = context.service_worker_target_mut(&target_id) else {
            return outputs;
        };
        target.update_version_status(browser_context_id, status)
    }) else {
        return outputs;
    };
    let service_worker_domain_sessions =
        service_worker::enabled_sessions_for_browser_context(conn, browser_context_id);
    if service_worker_domain_sessions.is_empty() {
        return outputs;
    }
    let Some(context) = conn.browser_context_by_id(browser_context_id) else {
        return outputs;
    };
    let Some(target_id) = context
        .service_worker_target_id_for_renderer_version(renderer_version_id)
        .map(str::to_owned)
    else {
        return outputs;
    };
    let Some(target) = context.service_worker_target(&target_id) else {
        return outputs;
    };
    let events = service_worker::version_updated_events_for_target(
        context,
        target,
        &service_worker_domain_sessions,
    );
    push_service_worker_version_events(&mut outputs, version, events);
    outputs
}

fn remove_shared_worker_target(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_instance_id: SharedWorkerInstanceId,
) -> TargetPreparedOutputs {
    remove_shared_worker_target_with_reason(
        conn,
        browser_context_id,
        renderer_instance_id,
        "Target closed",
    )
}

fn remove_shared_worker_target_with_reason(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_instance_id: SharedWorkerInstanceId,
    reason: &'static str,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let should_emit_destroyed = conn.has_any_target_discovery();
    let target_id = {
        let Some(context) = conn.browser_context_by_id(browser_context_id) else {
            return outputs;
        };
        let Some(target_id) = context
            .shared_worker_target_id_for_renderer_instance(renderer_instance_id)
            .map(str::to_owned)
        else {
            return outputs;
        };
        target_id
    };
    let destroyed_delta = should_emit_destroyed
        .then(|| conn.prepare_destroyed_target_host_delta(&target_id))
        .flatten();
    let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
        return outputs;
    };
    let Some(mut target) =
        context.remove_shared_worker_target_by_renderer_instance(renderer_instance_id)
    else {
        return outputs;
    };
    conn.remove_worker_target_host(&target_id);
    let session_ids = target.session_ids();
    let mut pending_await_direct_outputs = Vec::new();
    let mut pending_await_claimed_outputs = Vec::new();
    for session_id in &session_ids {
        let attachment = target
            .protocol_attachment_identity(browser_context_id, session_id)
            .expect("removed shared-worker session must retain its exact attachment identity");
        let mut pending_await_direct_events = Vec::new();
        let mut pending_await_claimed_events = Vec::new();
        conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
            &mut pending_await_direct_events,
            &mut pending_await_claimed_events,
            Some(session_id),
            reason,
        );
        CdpConnection::fail_pending_inspector_awaits_from_shared_worker_target_session_background_events_into(
            &mut pending_await_direct_events,
            &mut target,
            session_id,
            reason,
        );
        if !pending_await_direct_events.is_empty() {
            pending_await_direct_outputs.push((attachment.clone(), pending_await_direct_events));
        }
        if !pending_await_claimed_events.is_empty() {
            pending_await_claimed_outputs.push((attachment, pending_await_claimed_events));
        }
    }
    for (attachment, events) in pending_await_direct_outputs
        .into_iter()
        .chain(pending_await_claimed_outputs)
    {
        outputs
            .push(WorkerTargetLifecycleOutput::SharedWorkerAttachmentEvents { attachment, events });
    }
    for session_id in session_ids {
        let retirement = target
            .take_protocol_attachment_retirement(browser_context_id, &session_id)
            .expect("removed shared-worker session must transfer its attachment scope");
        outputs.push(WorkerTargetLifecycleOutput::SharedWorkerDetached {
            cleanup_plan: TargetSessionDetachCleanupPlan::new(
                target_id.clone(),
                session_id,
                None,
                None,
            ),
            retirement,
        });
    }
    if let Some(target_delta) = destroyed_delta {
        outputs.push(WorkerTargetLifecycleOutput::SharedWorkerDestroyed { target_delta });
    }
    outputs
}

fn remove_service_worker_target(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_version_id: u64,
    active_renderer_run: Option<RendererServiceWorkerRunIdentity>,
) -> TargetPreparedOutputs {
    remove_service_worker_target_with_reason(
        conn,
        browser_context_id,
        renderer_version_id,
        ServiceWorkerTargetRemovalAuthority::RendererDestroyed {
            active_renderer_run,
        },
        "Target closed",
    )
}

/// Selects which lifetime authority may retire a stable ServiceWorker target.
///
/// A renderer `Destroyed` event is run-scoped and must prove that the observed
/// run is still the target's active run. Browser-context disposal owns the
/// stable target itself, so it intentionally bypasses that run-currentness
/// guard after stopping all renderer workers. Keeping these cases typed avoids
/// using nested `Option` values to encode two different authorities.
enum ServiceWorkerTargetRemovalAuthority {
    RendererDestroyed {
        active_renderer_run: Option<RendererServiceWorkerRunIdentity>,
    },
    BrowserContextDisposal,
}

impl ServiceWorkerTargetRemovalAuthority {
    fn authorizes(self, target: &ServiceWorkerTargetState) -> bool {
        match self {
            Self::RendererDestroyed {
                active_renderer_run,
            } => target.observes_destroyed_active_run(active_renderer_run.as_ref()),
            Self::BrowserContextDisposal => true,
        }
    }
}

fn remove_service_worker_target_with_reason(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_version_id: u64,
    authority: ServiceWorkerTargetRemovalAuthority,
    reason: &'static str,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let should_emit_destroyed = conn.has_any_target_discovery();
    let service_worker_domain_sessions =
        service_worker::enabled_sessions_for_browser_context(conn, browser_context_id);
    let target_id = {
        let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
            return outputs;
        };
        let Some(target_id) = context
            .service_worker_target_id_for_renderer_version(renderer_version_id)
            .map(str::to_owned)
        else {
            return outputs;
        };
        let Some(target) = context.service_worker_target_mut(&target_id) else {
            return outputs;
        };
        if !authority.authorizes(target) {
            return outputs;
        }
        target_id
    };
    let destroyed_delta = should_emit_destroyed
        .then(|| conn.prepare_destroyed_target_host_delta(&target_id))
        .flatten();
    let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
        return outputs;
    };
    let Some(mut target) =
        context.remove_service_worker_target_by_renderer_version(renderer_version_id)
    else {
        return outputs;
    };
    let registration_deleted = !context
        .service_worker_targets
        .values()
        .any(|other| other.renderer_registration_id == target.renderer_registration_id);
    let version = target
        .version_identity(browser_context_id)
        .expect("removed service-worker target must retain its exact version identity");
    let run_retirement = target.take_current_run_retirement(browser_context_id);
    conn.remove_worker_target_host(&target_id);
    let deleted_events = service_worker::deleted_target_events(
        &target,
        registration_deleted,
        &service_worker_domain_sessions,
    );
    push_service_worker_version_events(&mut outputs, version.clone(), deleted_events);
    let session_ids = target.session_ids();
    let attachments = session_ids
        .iter()
        .map(|session_id| {
            (
                session_id.clone(),
                target
                    .protocol_attachment_identity(browser_context_id, session_id)
                    .expect("removed service-worker session must retain its exact attachment"),
            )
        })
        .collect::<Vec<_>>();
    for session_id in &session_ids {
        let mut pending_await_direct_events = Vec::new();
        let mut pending_await_claimed_events = Vec::new();
        conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
            &mut pending_await_direct_events,
            &mut pending_await_claimed_events,
            Some(session_id),
            reason,
        );
        pending_await_direct_events.extend(pending_await_claimed_events);
        let attachment = attachments
            .iter()
            .find(|(candidate, _)| candidate == session_id)
            .map(|(_, attachment)| attachment.clone())
            .expect("removed service-worker session must retain its prepared attachment");
        push_service_worker_attachment_events(
            &mut outputs,
            attachment,
            pending_await_direct_events,
        );
    }
    let mut target_pending_await_events = Vec::new();
    CdpConnection::fail_pending_inspector_awaits_from_service_worker_target_state_background_events_into(
        &mut target_pending_await_events,
        &mut target,
        reason,
    );
    push_service_worker_version_events(&mut outputs, version.clone(), target_pending_await_events);
    if let Some(retirement) = run_retirement {
        outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerRunRetired { retirement });
    }
    for session_id in session_ids {
        let retirement = target
            .take_protocol_attachment_retirement(browser_context_id, &session_id)
            .expect("removed service-worker session must transfer its attachment scope");
        outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerDetached {
            cleanup_plan: TargetSessionDetachCleanupPlan::new(
                target_id.clone(),
                session_id,
                None,
                None,
            ),
            retirement,
        });
    }
    let retirement = target
        .take_version_retirement(browser_context_id)
        .expect("removed service-worker target must transfer its version scope");
    outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerDestroyed {
        retirement,
        target_delta: destroyed_delta,
    });
    outputs
}

pub(super) async fn close_browser_context_worker_targets_for_dispose_async(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    reason: &'static str,
) -> Vec<BackgroundProtocolEvent> {
    let Some((renderer_runtime, shared_worker_ids, service_worker_ids)) = conn
        .browser_context_by_id(browser_context_id)
        .map(|context| {
            (
                context.renderer_runtime(),
                context
                    .shared_worker_targets
                    .keys()
                    .copied()
                    .collect::<Vec<_>>(),
                context
                    .service_worker_targets
                    .values()
                    .map(|target| target.renderer_version_id)
                    .collect::<Vec<_>>(),
            )
        })
    else {
        return Vec::new();
    };

    let mut outputs = TargetPreparedOutputs::default();
    for instance_id in shared_worker_ids {
        renderer_runtime.close_shared_worker_for_target_close(instance_id);
        outputs.extend(remove_shared_worker_target_with_reason(
            conn,
            browser_context_id,
            instance_id,
            reason,
        ));
    }
    if let Err(error) = renderer_runtime.stop_all_service_workers_for_devtools() {
        tracing::warn!(
            browser_context_id,
            error,
            "browser context disposal could not stop all renderer service workers"
        );
    }
    for version_id in service_worker_ids {
        outputs.extend(remove_service_worker_target_with_reason(
            conn,
            browser_context_id,
            version_id,
            ServiceWorkerTargetRemovalAuthority::BrowserContextDisposal,
            reason,
        ));
    }

    worker_target_removal_background_events_async(conn, outputs).await
}

async fn worker_target_removal_background_events_async(
    conn: &mut CdpConnection,
    outputs: TargetPreparedOutputs,
) -> Vec<BackgroundProtocolEvent> {
    let mut command_context = crate::conn::CommandDispatchContext::default();
    let mut prepared_outputs =
        ProtocolOutputPayloads::from_slot(TargetPreparedOutputSlot::from_outputs(outputs));
    emit_target_lifecycle_events(
        conn,
        &mut ProtocolOutputProjectionContext::new(None, &mut command_context),
        Some(&mut prepared_outputs),
    )
    .await;
    command_context.take_protocol_events()
}

pub(super) async fn close_shared_worker_target_for_target_close_async(
    conn: &mut CdpConnection,
    target_id: &str,
    command_context: &mut crate::conn::CommandDispatchContext,
) -> bool {
    let Some((browser_context_id, renderer_runtime, instance_id)) =
        conn.browser_context.as_ref().and_then(|context| {
            let target = context.shared_worker_target(target_id)?;
            Some((
                context.id.clone(),
                context.renderer_runtime(),
                target.renderer_instance_id,
            ))
        })
    else {
        return false;
    };

    renderer_runtime.close_shared_worker_for_target_close(instance_id);
    let outputs = remove_shared_worker_target(conn, &browser_context_id, instance_id);
    let mut prepared_outputs =
        ProtocolOutputPayloads::from_slot(TargetPreparedOutputSlot::from_outputs(outputs));
    emit_target_lifecycle_events(
        conn,
        &mut ProtocolOutputProjectionContext::new(None, command_context),
        Some(&mut prepared_outputs),
    )
    .await;
    true
}

pub(super) async fn close_dedicated_worker_target_for_target_close_async(
    conn: &mut CdpConnection,
    target_id: &str,
    command_context: &mut crate::conn::CommandDispatchContext,
) -> bool {
    let Some((browser_context_id, renderer_runtime, instance_id)) =
        conn.browser_context.as_ref().and_then(|context| {
            let target = context.dedicated_worker_target(target_id)?;
            Some((
                context.id.clone(),
                context.renderer_runtime(),
                target.renderer_instance_id,
            ))
        })
    else {
        return false;
    };

    renderer_runtime.close_dedicated_worker_for_devtools(instance_id);
    let outputs = prepare_dedicated_worker_target_retirement(
        conn,
        &browser_context_id,
        instance_id,
        DedicatedWorkerRetirementCause::OwnerRetired,
    );
    let mut prepared_outputs =
        ProtocolOutputPayloads::from_slot(TargetPreparedOutputSlot::from_outputs(outputs));
    emit_target_lifecycle_events(
        conn,
        &mut ProtocolOutputProjectionContext::new(None, command_context),
        Some(&mut prepared_outputs),
    )
    .await;
    true
}

fn record_shared_worker_target_console_message(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_instance_id: SharedWorkerInstanceId,
    message: RendererSharedWorkerConsoleMessage,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
        return outputs;
    };
    let Some(target_id) = context
        .shared_worker_target_id_for_renderer_instance(renderer_instance_id)
        .map(str::to_owned)
    else {
        // Worker inspector messages can race with target destruction. Once the
        // renderer instance has no CDP target, late messages are stale and must
        // not recreate target state or replay into a later fresh worker.
        return outputs;
    };
    let Some(target) = context.shared_worker_target_mut(&target_id) else {
        return outputs;
    };
    target.record_console_message(message);
    for session_id in target.session_ids() {
        let attachment = target
            .protocol_attachment_identity(browser_context_id, &session_id)
            .expect("shared-worker output session must retain its exact attachment identity");
        let console_messages = target.pending_console_domain_messages(&session_id).to_vec();
        if !console_messages.is_empty() {
            outputs.push(WorkerTargetLifecycleOutput::SharedWorkerConsoleMessages {
                attachment: attachment.clone(),
                messages: console_messages,
                console_end: target.console_message_count(),
            });
        }
        let runtime_messages = target
            .pending_runtime_console_messages(&session_id)
            .to_vec();
        if !runtime_messages.is_empty() {
            outputs.push(
                WorkerTargetLifecycleOutput::SharedWorkerRuntimeConsoleMessages {
                    attachment,
                    messages: runtime_messages,
                    console_end: target.console_message_count(),
                },
            );
        }
    }
    outputs
}

fn record_shared_worker_target_runtime_inspector_messages(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_instance_id: SharedWorkerInstanceId,
    inspector_session_id: Option<String>,
    messages: Vec<RendererRuntimeInspectorMessage>,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    if messages.is_empty() {
        return outputs;
    }
    let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
        return outputs;
    };
    let Some(target_id) = context
        .shared_worker_target_id_for_renderer_instance(renderer_instance_id)
        .map(str::to_owned)
    else {
        return outputs;
    };
    let Some(target) = context.shared_worker_target_mut(&target_id) else {
        return outputs;
    };
    let session_ids = if let Some(session_id) = inspector_session_id {
        if !target.is_session(&session_id) {
            return outputs;
        }
        vec![session_id]
    } else {
        target.session_ids()
    };
    for session_id in session_ids {
        let attachment = target
            .protocol_attachment_identity(browser_context_id, &session_id)
            .expect("shared-worker inspector route must retain its exact attachment identity");
        outputs.push(
            WorkerTargetLifecycleOutput::SharedWorkerRuntimeInspectorMessages {
                attachment,
                messages: messages.clone(),
            },
        );
    }
    outputs
}

fn record_service_worker_target_console_message(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_version_id: u64,
    renderer_run: RendererServiceWorkerRunIdentity,
    message: RendererServiceWorkerConsoleMessage,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
        return outputs;
    };
    let Some(target_id) = context
        .service_worker_target_id_for_renderer_version(renderer_version_id)
        .map(str::to_owned)
    else {
        return outputs;
    };
    let Some(target) = context.service_worker_target_mut(&target_id) else {
        return outputs;
    };
    let Some(run) = target.observe_worker_run(browser_context_id, renderer_run) else {
        return outputs;
    };
    target.record_console_message(message.message, message.args, message.stack);
    for session_id in target.session_ids() {
        let Some(attachment) = target.protocol_attachment_identity(browser_context_id, &session_id)
        else {
            continue;
        };
        let runtime = TargetServiceWorkerRuntimeAttachmentIdentity::new(attachment, run.clone());
        let console_messages = target.pending_console_domain_messages(&session_id).to_vec();
        if !console_messages.is_empty() {
            let console_end = target.console_message_count();
            target.mark_console_domain_emitted(&session_id, console_end);
            outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerConsoleMessages {
                runtime: runtime.clone(),
                messages: console_messages,
                console_end,
            });
        }
        let runtime_messages = target
            .pending_runtime_console_messages(&session_id)
            .to_vec();
        if !runtime_messages.is_empty() {
            let console_end = target.console_message_count();
            target.mark_runtime_console_emitted(&session_id, console_end);
            outputs.push(
                WorkerTargetLifecycleOutput::ServiceWorkerRuntimeConsoleMessages {
                    runtime,
                    messages: runtime_messages,
                    console_end,
                },
            );
        }
    }
    outputs
}

fn record_service_worker_target_exception_message(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_version_id: u64,
    renderer_run: RendererServiceWorkerRunIdentity,
    message: RendererServiceWorkerExceptionMessage,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let service_worker_domain_sessions =
        service_worker::enabled_sessions_for_browser_context(conn, browser_context_id);
    let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
        return outputs;
    };
    let Some(target_id) = context
        .service_worker_target_id_for_renderer_version(renderer_version_id)
        .map(str::to_owned)
    else {
        return outputs;
    };
    let Some(target) = context.service_worker_target_mut(&target_id) else {
        return outputs;
    };
    let Some(run) = target.observe_worker_run(browser_context_id, renderer_run) else {
        return outputs;
    };
    let service_worker_error_events =
        service_worker::error_reported_events(target, &message, &service_worker_domain_sessions);
    target.record_exception_message(message);
    push_service_worker_run_events(&mut outputs, run.clone(), service_worker_error_events);
    for session_id in target.session_ids() {
        let Some(attachment) = target.protocol_attachment_identity(browser_context_id, &session_id)
        else {
            continue;
        };
        let exception_start = target
            .exception_message_count()
            .saturating_sub(target.pending_runtime_exception_messages(&session_id).len());
        let exception_messages = target
            .pending_runtime_exception_messages(&session_id)
            .to_vec();
        if !exception_messages.is_empty() {
            let exception_end = target.exception_message_count();
            target.mark_runtime_exception_emitted(&session_id, exception_end);
            outputs.push(
                WorkerTargetLifecycleOutput::ServiceWorkerRuntimeExceptionMessages {
                    runtime: TargetServiceWorkerRuntimeAttachmentIdentity::new(
                        attachment,
                        run.clone(),
                    ),
                    messages: exception_messages,
                    exception_start,
                    exception_end,
                },
            );
        }
    }
    outputs
}

fn record_service_worker_target_fetch_diagnostic(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_version_id: u64,
    renderer_run: RendererServiceWorkerRunIdentity,
    diagnostic: RendererServiceWorkerFetchDiagnostic,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
        return outputs;
    };
    let Some(target_id) = context
        .service_worker_target_id_for_renderer_version(renderer_version_id)
        .map(str::to_owned)
    else {
        return outputs;
    };
    let Some(target) = context.service_worker_target_mut(&target_id) else {
        return outputs;
    };
    let Some(run) = target.observe_worker_run(browser_context_id, renderer_run) else {
        return outputs;
    };
    target.record_fetch_diagnostic(diagnostic);
    for session_id in target.session_ids() {
        let Some(attachment) = target.protocol_attachment_identity(browser_context_id, &session_id)
        else {
            continue;
        };
        let diagnostics = target.pending_fetch_diagnostics(&session_id).to_vec();
        if !diagnostics.is_empty() {
            let diagnostic_end = target.fetch_diagnostic_count();
            let diagnostic_start = diagnostic_end.saturating_sub(diagnostics.len());
            target.mark_fetch_diagnostics_emitted(&session_id, diagnostic_end);
            outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerFetchDiagnostics {
                runtime: TargetServiceWorkerRuntimeAttachmentIdentity::new(attachment, run.clone()),
                diagnostics,
                diagnostic_start,
                diagnostic_end,
            });
        }
    }
    outputs
}

fn record_service_worker_target_runtime_inspector_messages(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_version_id: u64,
    renderer_run: RendererServiceWorkerRunIdentity,
    inspector_session_id: Option<String>,
    messages: Vec<RendererRuntimeInspectorMessage>,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    if messages.is_empty() {
        return outputs;
    }
    let runtimes = {
        let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
            return outputs;
        };
        let Some(target_id) = context
            .service_worker_target_id_for_renderer_version(renderer_version_id)
            .map(str::to_owned)
        else {
            return outputs;
        };
        let Some(target) = context.service_worker_target_mut(&target_id) else {
            return outputs;
        };
        let Some(run) = target.observe_worker_run(browser_context_id, renderer_run) else {
            return outputs;
        };
        let session_ids = if let Some(session_id) = inspector_session_id {
            if !target.is_session(&session_id) {
                return outputs;
            }
            vec![session_id]
        } else {
            target.session_ids()
        };
        session_ids
            .into_iter()
            .filter_map(|session_id| {
                Some(TargetServiceWorkerRuntimeAttachmentIdentity::new(
                    target.protocol_attachment_identity(browser_context_id, &session_id)?,
                    run.clone(),
                ))
            })
            .collect::<Vec<_>>()
    };
    for runtime in runtimes {
        let session_id = runtime.session_id().to_owned();
        let mut response_events = Vec::new();
        let mut background_events = Vec::new();
        let current_response_seen = route_worker_runtime_inspector_messages_into(
            conn,
            messages.clone(),
            &session_id,
            &mut response_events,
            &mut background_events,
        );
        debug_assert!(!current_response_seen);

        let mut pending_runtime_console = None;
        let mut pending_runtime_exceptions = None;
        if let Some(target) = exact_service_worker_runtime_target_mut(conn, &runtime) {
            let pending_console = target
                .pending_runtime_console_messages(&session_id)
                .to_vec();
            if !pending_console.is_empty() {
                let console_end = target.console_message_count();
                target.mark_runtime_console_emitted(&session_id, console_end);
                pending_runtime_console = Some((pending_console, console_end));
            }
            let exception_start = target
                .exception_message_count()
                .saturating_sub(target.pending_runtime_exception_messages(&session_id).len());
            let pending_exceptions = target
                .pending_runtime_exception_messages(&session_id)
                .to_vec();
            if !pending_exceptions.is_empty() {
                let exception_end = target.exception_message_count();
                target.mark_runtime_exception_emitted(&session_id, exception_end);
                pending_runtime_exceptions =
                    Some((pending_exceptions, exception_start, exception_end));
            }
        }
        outputs.push(
            WorkerTargetLifecycleOutput::ServiceWorkerRuntimeInspectorMessages {
                runtime,
                background_events,
                response_events,
                pending_runtime_console,
                pending_runtime_exceptions,
            },
        );
    }
    outputs
}

pub(in crate::domains) async fn project_worker_target_output_async(
    output: ProtocolOutputSlot,
    conn: &mut CdpConnection,
    context: &mut ProtocolOutputProjectionContext<'_>,
    prepared_outputs: Option<&mut ProtocolOutputPayloads>,
) {
    match output {
        ProtocolOutputSlot::SharedWorkerTargetLifecycle
        | ProtocolOutputSlot::ServiceWorkerTargetLifecycle
        | ProtocolOutputSlot::DedicatedWorkerTargetLifecycle => {}
        _ => panic!("non-Target output routed through the Target projector: {output:?}"),
    }
    emit_target_lifecycle_events(conn, context, prepared_outputs).await;
}

async fn emit_target_lifecycle_events(
    conn: &mut CdpConnection,
    context: &mut ProtocolOutputProjectionContext<'_>,
    prepared_outputs: Option<&mut ProtocolOutputPayloads>,
) {
    let Some(events) = prepared_outputs
        .and_then(ProtocolOutputPayloads::target_mut)
        .and_then(TargetPreparedOutputSlot::take_worker_target_lifecycle_outputs)
    else {
        return;
    };
    let mut side_effects = events::TargetProtocolSideEffects::default();
    for event in events {
        let event = match commit_dedicated_worker_retirement_output_async(conn, event).await {
            Ok(events) => {
                side_effects.extend_background_events(events);
                continue;
            }
            Err(event) => event,
        };
        match event {
            WorkerTargetLifecycleOutput::DedicatedWorkerEvents {
                browser_context_id,
                renderer_instance_id,
                target_id,
                events,
            } => {
                if dedicated_worker_target_is_current(
                    conn,
                    &browser_context_id,
                    renderer_instance_id,
                    &target_id,
                ) {
                    side_effects.extend_background_events(events);
                }
            }
            WorkerTargetLifecycleOutput::DedicatedWorkerConsoleMessages {
                browser_context_id,
                renderer_instance_id,
                target_id,
                session_id,
                console_messages,
                runtime_messages,
                console_end,
            } => {
                if !dedicated_worker_target_is_current(
                    conn,
                    &browser_context_id,
                    renderer_instance_id,
                    &target_id,
                ) || !conn
                    .browser_context_by_id(&browser_context_id)
                    .and_then(|context| context.dedicated_worker_targets.get(&renderer_instance_id))
                    .is_some_and(|target| target.is_session(&session_id))
                {
                    continue;
                }
                side_effects.extend_background_events(console_message_added_events(
                    &session_id,
                    &console_messages,
                ));
                side_effects.extend_background_events(runtime_console_api_called_events(
                    &session_id,
                    &runtime_messages,
                ));
                if let Some(target) = conn
                    .browser_context_by_id_mut(&browser_context_id)
                    .and_then(|context| {
                        context
                            .dedicated_worker_targets
                            .get_mut(&renderer_instance_id)
                    })
                    .filter(|target| target.target_id == target_id)
                {
                    if !console_messages.is_empty() {
                        target.mark_console_domain_emitted(&session_id, console_end);
                    }
                    if !runtime_messages.is_empty() {
                        target.mark_runtime_console_emitted(&session_id, console_end);
                    }
                }
            }
            WorkerTargetLifecycleOutput::DedicatedWorkerCreated {
                browser_context_id,
                renderer_instance_id,
                target_delta,
            } => {
                if dedicated_worker_target_is_current(
                    conn,
                    &browser_context_id,
                    renderer_instance_id,
                    target_delta.target_id(),
                ) {
                    side_effects.extend_background_events(
                        conn.prepared_target_host_delta_event_plan(target_delta),
                    );
                }
            }
            WorkerTargetLifecycleOutput::DedicatedWorkerInfoChanged {
                browser_context_id,
                renderer_instance_id,
                target_id,
                target_delta,
            } => {
                if dedicated_worker_target_is_current(
                    conn,
                    &browser_context_id,
                    renderer_instance_id,
                    &target_id,
                ) {
                    side_effects.extend_background_events(
                        conn.prepared_target_host_delta_event_plan(target_delta),
                    );
                }
            }
            WorkerTargetLifecycleOutput::DedicatedWorkerAttached {
                browser_context_id,
                renderer_instance_id,
                target_id,
                session_id,
                prepared_attach,
            } => {
                let current = conn
                    .browser_context_by_id(&browser_context_id)
                    .and_then(|context| context.dedicated_worker_targets.get(&renderer_instance_id))
                    .is_some_and(|target| {
                        target.target_id == target_id && target.is_session(&session_id)
                    });
                if current {
                    side_effects.extend_background_events(
                        conn.commit_prepared_dedicated_worker_attach_event_plan(prepared_attach),
                    );
                }
            }
            WorkerTargetLifecycleOutput::DedicatedWorkerDetached { .. }
            | WorkerTargetLifecycleOutput::DedicatedWorkerDestroyed { .. } => {
                unreachable!("DedicatedWorker retirement outputs are committed before projection")
            }
            WorkerTargetLifecycleOutput::SharedWorkerAttachmentEvents { attachment, events } => {
                if attachment.is_current() {
                    side_effects.extend_background_events(events);
                }
            }
            WorkerTargetLifecycleOutput::SharedWorkerCreated { target_delta } => {
                side_effects.extend_background_events(
                    conn.prepared_target_host_delta_event_plan(target_delta),
                );
            }
            WorkerTargetLifecycleOutput::SharedWorkerAttached {
                attachment,
                prepared_attach,
            } => {
                if !attachment.is_current() {
                    continue;
                }
                side_effects.extend_background_events(
                    conn.commit_prepared_attach_event_plan(prepared_attach),
                );
            }
            WorkerTargetLifecycleOutput::ServiceWorkerVersionEvents { version, events } => {
                if version.is_current() {
                    side_effects.extend_background_events(events);
                }
            }
            WorkerTargetLifecycleOutput::ServiceWorkerAttachmentEvents { attachment, events } => {
                if attachment.is_current() {
                    side_effects.extend_background_events(events);
                }
            }
            WorkerTargetLifecycleOutput::ServiceWorkerRunEvents { run, events } => {
                if run.is_current() {
                    side_effects.extend_background_events(events);
                }
            }
            WorkerTargetLifecycleOutput::ServiceWorkerRuntimeEvents { runtime, events } => {
                if runtime.is_current() {
                    side_effects.extend_background_events(events);
                }
            }
            WorkerTargetLifecycleOutput::ServiceWorkerCreated {
                version,
                target_delta,
            } => {
                if version.is_current() {
                    side_effects.extend_background_events(
                        conn.prepared_target_host_delta_event_plan(target_delta),
                    );
                }
            }
            WorkerTargetLifecycleOutput::ServiceWorkerAttached {
                attachment,
                prepared_attach,
            } => {
                if attachment.is_current() {
                    side_effects.extend_background_events(
                        conn.commit_prepared_attach_event_plan(prepared_attach),
                    );
                }
            }
            WorkerTargetLifecycleOutput::SharedWorkerDetached {
                retirement,
                cleanup_plan,
            } => {
                if !retirement.is_current() {
                    continue;
                }
                assert_eq!(
                    cleanup_plan.target_id(),
                    retirement.identity().target_id(),
                    "shared-worker detach plan must retain its exact target"
                );
                assert_eq!(
                    cleanup_plan.session_id(),
                    retirement.identity().session_id(),
                    "shared-worker detach plan must retain its exact attachment"
                );
                let event_plan = conn
                    .detach_session_with_binding_cleanup_event_plan_async(cleanup_plan)
                    .await;
                side_effects.extend_background_events(event_plan);
                retirement.retire();
            }
            WorkerTargetLifecycleOutput::ServiceWorkerDetached {
                retirement,
                cleanup_plan,
            } => {
                if !retirement.is_current() {
                    continue;
                }
                assert_eq!(
                    cleanup_plan.target_id(),
                    retirement.identity().target_id(),
                    "service-worker detach plan must retain its exact version target"
                );
                assert_eq!(
                    cleanup_plan.session_id(),
                    retirement.identity().session_id(),
                    "service-worker detach plan must retain its exact attachment"
                );
                let event_plan = conn
                    .detach_session_with_binding_cleanup_event_plan_async(cleanup_plan)
                    .await;
                side_effects.extend_background_events(event_plan);
                retirement.retire();
            }
            WorkerTargetLifecycleOutput::SharedWorkerDestroyed { target_delta } => {
                side_effects.extend_background_events(
                    conn.prepared_target_host_delta_event_plan(target_delta),
                );
            }
            WorkerTargetLifecycleOutput::ServiceWorkerRunRetired { retirement } => {
                assert!(
                    retirement.is_current(),
                    "service-worker run retirement must be consumed exactly once in source order"
                );
                retirement.retire();
            }
            WorkerTargetLifecycleOutput::ServiceWorkerDestroyed {
                retirement,
                target_delta,
            } => {
                assert!(
                    retirement.is_current(),
                    "service-worker version retirement must be consumed exactly once"
                );
                if let Some(target_delta) = target_delta {
                    assert_eq!(
                        target_delta.target_id(),
                        retirement.identity().target_id(),
                        "service-worker destruction must retain its exact version target"
                    );
                    side_effects.extend_background_events(
                        conn.prepared_target_host_delta_event_plan(target_delta),
                    );
                }
                retirement.retire();
            }
            WorkerTargetLifecycleOutput::ServiceWorkerConsoleMessages {
                runtime,
                messages,
                console_end: _,
            } => {
                if runtime.is_current() {
                    side_effects.extend_background_events(console_message_added_events(
                        runtime.session_id(),
                        &messages,
                    ));
                }
            }
            WorkerTargetLifecycleOutput::SharedWorkerConsoleMessages {
                attachment,
                messages,
                console_end,
            } => {
                if !attachment.is_current() {
                    continue;
                }
                side_effects.extend_background_events(console_message_added_events(
                    attachment.session_id(),
                    &messages,
                ));
                mark_exact_shared_worker_console_domain_emitted(conn, &attachment, console_end);
            }
            WorkerTargetLifecycleOutput::ServiceWorkerRuntimeConsoleMessages {
                runtime,
                messages,
                console_end: _,
            } => {
                if runtime.is_current() {
                    side_effects.extend_background_events(runtime_console_api_called_events(
                        runtime.session_id(),
                        &messages,
                    ));
                }
            }
            WorkerTargetLifecycleOutput::SharedWorkerRuntimeConsoleMessages {
                attachment,
                messages,
                console_end,
            } => {
                if !attachment.is_current() {
                    continue;
                }
                side_effects.extend_background_events(runtime_console_api_called_events(
                    attachment.session_id(),
                    &messages,
                ));
                mark_exact_shared_worker_runtime_console_emitted(conn, &attachment, console_end);
            }
            WorkerTargetLifecycleOutput::ServiceWorkerRuntimeExceptionMessages {
                runtime,
                messages,
                exception_start,
                exception_end: _,
            } => {
                if runtime.is_current() {
                    side_effects.extend_background_events(runtime_exception_thrown_events(
                        runtime.session_id(),
                        &messages,
                        exception_start,
                    ));
                }
            }
            WorkerTargetLifecycleOutput::ServiceWorkerFetchDiagnostics {
                runtime,
                diagnostics,
                diagnostic_start,
                diagnostic_end: _,
            } => {
                if runtime.is_current() {
                    side_effects.extend_background_events(service_worker_fetch_diagnostic_events(
                        runtime.session_id(),
                        runtime.target_id(),
                        &diagnostics,
                        diagnostic_start,
                    ));
                }
            }
            WorkerTargetLifecycleOutput::ServiceWorkerRuntimeInspectorMessages {
                runtime,
                background_events,
                response_events,
                pending_runtime_console,
                pending_runtime_exceptions,
            } => {
                if !runtime.is_current() {
                    continue;
                }
                let session_id = runtime.session_id();
                side_effects.extend_background_events(background_events);
                if service_worker_runtime_is_registry_current(conn, &runtime) {
                    replay_shared_worker_runtime_bindings_for_session_async(conn, Some(session_id))
                        .await;
                }
                side_effects.extend_background_events(response_events);
                if let Some((messages, _console_end)) = pending_runtime_console {
                    side_effects.extend_background_events(runtime_console_api_called_events(
                        session_id, &messages,
                    ));
                }
                if let Some((messages, exception_start, _exception_end)) =
                    pending_runtime_exceptions
                {
                    side_effects.extend_background_events(runtime_exception_thrown_events(
                        session_id,
                        &messages,
                        exception_start,
                    ));
                }
            }
            WorkerTargetLifecycleOutput::SharedWorkerRuntimeInspectorMessages {
                attachment,
                messages,
            } => {
                if !attachment.is_current() {
                    continue;
                }
                let session_id = attachment.session_id();
                let mut response_events = Vec::new();
                let mut background_events = Vec::new();
                let current_response_seen = route_worker_runtime_inspector_messages_into(
                    conn,
                    messages,
                    session_id,
                    &mut response_events,
                    &mut background_events,
                );
                debug_assert!(!current_response_seen);
                side_effects.extend_background_events(background_events);
                let pending_runtime_console =
                    exact_shared_worker_pending_runtime_console(conn, &attachment);
                replay_shared_worker_runtime_bindings_for_session_async(conn, Some(session_id))
                    .await;
                side_effects.extend_background_events(response_events);
                if let Some((messages, console_end)) = pending_runtime_console {
                    side_effects.extend_background_events(runtime_console_api_called_events(
                        session_id, &messages,
                    ));
                    mark_exact_shared_worker_runtime_console_emitted(
                        conn,
                        &attachment,
                        console_end,
                    );
                }
            }
            WorkerTargetLifecycleOutput::DedicatedWorkerRuntimeInspectorMessages {
                browser_context_id,
                renderer_instance_id,
                target_id,
                session_id,
                messages,
            } => {
                if !dedicated_worker_target_is_current(
                    conn,
                    &browser_context_id,
                    renderer_instance_id,
                    &target_id,
                ) {
                    continue;
                }
                let mut response_events = Vec::new();
                let mut background_events = Vec::new();
                let current_response_seen = route_worker_runtime_inspector_messages_into(
                    conn,
                    messages,
                    &session_id,
                    &mut response_events,
                    &mut background_events,
                );
                debug_assert!(!current_response_seen);
                side_effects.extend_background_events(background_events);
                replay_shared_worker_runtime_bindings_for_session_async(conn, Some(&session_id))
                    .await;
                side_effects.extend_background_events(response_events);
            }
        }
    }
    for event in side_effects.into_background_events() {
        context.command.push_protocol_event(event);
    }
}

fn route_worker_runtime_inspector_messages_into(
    conn: &mut CdpConnection,
    messages: Vec<RendererRuntimeInspectorMessage>,
    session_id: &str,
    response_events: &mut Vec<BackgroundProtocolEvent>,
    background_events: &mut Vec<BackgroundProtocolEvent>,
) -> bool {
    conn.route_renderer_runtime_inspector_messages_with_background_events_into(
        messages,
        None,
        Some(session_id),
        response_events,
        background_events,
    )
}

fn console_message_added_events(
    session_id: &str,
    messages: &[RuntimeConsoleMessageSnapshot],
) -> Vec<BackgroundProtocolEvent> {
    messages
        .iter()
        .map(|message| {
            let (level, text) = console_message_level_and_text(&message.message);
            console_message_added_background_event(Some(session_id), "console-api", level, text, "")
        })
        .collect()
}

fn runtime_console_api_called_events(
    session_id: &str,
    messages: &[RuntimeConsoleMessageSnapshot],
) -> Vec<BackgroundProtocolEvent> {
    let base_timestamp = monotonic_timestamp_seconds();
    messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            let (console_type, text) = runtime_console_message_type_and_text(&message.message);
            runtime_console_api_called_background_event(
                Some(session_id),
                None,
                console_type,
                text,
                &message.args,
                message.stack.as_deref(),
                message.execution_context_id,
                base_timestamp + ((index + 1) as f64 * 0.000_001),
            )
        })
        .collect()
}

fn runtime_exception_thrown_events(
    session_id: &str,
    messages: &[ServiceWorkerRuntimeExceptionSnapshot],
    exception_start: usize,
) -> Vec<BackgroundProtocolEvent> {
    let base_timestamp = monotonic_timestamp_seconds();
    messages
        .iter()
        .enumerate()
        .map(|(offset, message)| {
            let exception_index = exception_start + offset;
            runtime_exception_thrown_background_event(
                Some(session_id),
                None,
                &message.message.message,
                &message.message.filename,
                message.execution_context_id,
                exception_index,
                base_timestamp + ((offset + 1) as f64 * 0.000_001),
                Some(u64::from(message.message.lineno.saturating_sub(1))),
                Some(u64::from(message.message.colno.saturating_sub(1))),
            )
        })
        .collect()
}

fn service_worker_fetch_diagnostic_events(
    session_id: &str,
    target_id: &str,
    diagnostics: &[RendererServiceWorkerFetchDiagnostic],
    diagnostic_start: usize,
) -> Vec<BackgroundProtocolEvent> {
    let mut events = Vec::new();
    emit_service_worker_fetch_diagnostic_events(
        &mut events,
        session_id,
        target_id,
        diagnostics,
        diagnostic_start,
    );
    events
}

fn emit_service_worker_fetch_diagnostic_events(
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: &str,
    target_id: &str,
    diagnostics: &[RendererServiceWorkerFetchDiagnostic],
    diagnostic_start: usize,
) {
    let base_timestamp = monotonic_timestamp_seconds();
    for (index, diagnostic) in diagnostics.iter().enumerate() {
        let timestamp = base_timestamp + ((index + 1) as f64 * 0.000_001);
        let request_id = service_worker_fetch_diagnostic_request_id(
            target_id,
            diagnostic.internal_id,
            diagnostic_start + index,
        );
        let loader_id = service_worker_fetch_diagnostic_loader_id(target_id);
        let resource_type = service_worker_fetch_diagnostic_resource_type(&diagnostic.destination);
        let request_url = parse_service_worker_fetch_diagnostic_url(&diagnostic.request_url);
        let document_url =
            Url::parse(&diagnostic.document_url).unwrap_or_else(|_| request_url.clone());

        let request_event_index = out.len();
        network::emit_request_will_be_sent(
            out,
            Some(session_id),
            &request_id,
            target_id,
            &loader_id,
            timestamp,
            &document_url,
            &request_url,
            &diagnostic.method,
            diagnostic.request_body.as_deref(),
            &diagnostic.request_headers,
            resource_type,
            SubresourceRequestInitiatorType::Other,
            None,
            false,
            None,
            &[],
        );
        tag_service_worker_fetch_diagnostic_event(out.get_mut(request_event_index), diagnostic);

        match &diagnostic.result {
            RendererServiceWorkerFetchDiagnosticResult::Fallback => {
                let failed_event_index = out.len();
                network::emit_loading_failed(
                    out,
                    Some(session_id),
                    &request_id,
                    target_id,
                    &loader_id,
                    timestamp + 0.000_000_5,
                    "ServiceWorkerFallback",
                    resource_type,
                );
                tag_service_worker_fetch_diagnostic_event(
                    out.get_mut(failed_event_index),
                    diagnostic,
                );
            }
            RendererServiceWorkerFetchDiagnosticResult::Response {
                final_url,
                status,
                status_text,
                response_headers,
                body_len,
            } => {
                let final_url = Url::parse(final_url).unwrap_or_else(|_| request_url.clone());
                let response_event_index = out.len();
                network::emit_response_received(
                    out,
                    Some(session_id),
                    &request_id,
                    target_id,
                    &loader_id,
                    timestamp + 0.000_000_5,
                    &final_url,
                    *status,
                    Some(status_text),
                    response_headers,
                    &[],
                    *body_len,
                    false,
                    None,
                    false,
                    resource_type,
                    &[],
                    None,
                );
                tag_service_worker_fetch_diagnostic_event(
                    out.get_mut(response_event_index),
                    diagnostic,
                );
                if let Some(response) = out
                    .get_mut(response_event_index)
                    .and_then(BackgroundProtocolEvent::protocol_params_mut)
                    .and_then(|params| params.get_mut("response"))
                {
                    response["fromServiceWorker"] = json!(true);
                }
                let body_finished_event_start = out.len();
                network::emit_body_finished(
                    out,
                    Some(session_id),
                    &request_id,
                    target_id,
                    &loader_id,
                    timestamp + 0.000_001,
                    *body_len,
                    resource_type,
                );
                for event in &mut out[body_finished_event_start..] {
                    tag_service_worker_fetch_diagnostic_event(Some(event), diagnostic);
                }
            }
            RendererServiceWorkerFetchDiagnosticResult::Failure { message } => {
                let failed_event_index = out.len();
                network::emit_loading_failed(
                    out,
                    Some(session_id),
                    &request_id,
                    target_id,
                    &loader_id,
                    timestamp + 0.000_000_5,
                    message,
                    resource_type,
                );
                tag_service_worker_fetch_diagnostic_event(
                    out.get_mut(failed_event_index),
                    diagnostic,
                );
            }
        }
    }
}

fn service_worker_fetch_diagnostic_request_id(
    target_id: &str,
    internal_id: u64,
    diagnostic_index: usize,
) -> String {
    format!("{target_id}.sw-fetch.{internal_id}.{diagnostic_index}")
}

fn service_worker_fetch_diagnostic_loader_id(target_id: &str) -> String {
    format!("{target_id}.service-worker")
}

fn parse_service_worker_fetch_diagnostic_url(value: &str) -> Url {
    Url::parse(value).unwrap_or_else(|_| Url::parse("about:blank").expect("valid fallback URL"))
}

fn service_worker_runtime_is_registry_current(
    conn: &CdpConnection,
    runtime: &TargetServiceWorkerRuntimeAttachmentIdentity,
) -> bool {
    if !runtime.is_current() {
        return false;
    }
    let attachment = runtime.attachment();
    let Some(target) = conn
        .browser_context_by_id(attachment.browser_context_id())
        .and_then(|context| context.service_worker_target(attachment.target_id()))
    else {
        return false;
    };
    target.observes_runtime_identity(attachment.browser_context_id(), runtime)
}

fn exact_service_worker_runtime_target_mut<'a>(
    conn: &'a mut CdpConnection,
    runtime: &TargetServiceWorkerRuntimeAttachmentIdentity,
) -> Option<&'a mut ServiceWorkerTargetState> {
    if !runtime.is_current() {
        return None;
    }
    let attachment = runtime.attachment();
    let target = conn
        .browser_context_by_id_mut(attachment.browser_context_id())?
        .service_worker_target_mut(attachment.target_id())?;
    let is_exact = target.observes_runtime_identity(attachment.browser_context_id(), runtime);
    is_exact.then_some(target)
}

fn tag_service_worker_fetch_diagnostic_event(
    event: Option<&mut BackgroundProtocolEvent>,
    diagnostic: &RendererServiceWorkerFetchDiagnostic,
) {
    let Some(params) = event.and_then(BackgroundProtocolEvent::protocol_params_mut) else {
        return;
    };
    params["__moliServiceWorkerFetchDiagnostic"] = json!(true);
    params["__moliServiceWorkerFetchInternalId"] = json!(diagnostic.internal_id);
    params["__moliServiceWorkerFetchResult"] =
        json!(service_worker_fetch_diagnostic_result_name(diagnostic));
}

fn service_worker_fetch_diagnostic_result_name(
    diagnostic: &RendererServiceWorkerFetchDiagnostic,
) -> &'static str {
    match &diagnostic.result {
        RendererServiceWorkerFetchDiagnosticResult::Fallback => "fallback",
        RendererServiceWorkerFetchDiagnosticResult::Response { .. } => "response",
        RendererServiceWorkerFetchDiagnosticResult::Failure { .. } => "failure",
    }
}

fn service_worker_fetch_diagnostic_resource_type(destination: &str) -> DevToolsNetworkResourceType {
    match destination {
        "document" | "iframe" | "frame" => DevToolsNetworkResourceType::Document,
        "style" => DevToolsNetworkResourceType::Stylesheet,
        "image" => DevToolsNetworkResourceType::Image,
        "font" => DevToolsNetworkResourceType::Font,
        "audio" | "video" => DevToolsNetworkResourceType::Media,
        "script" | "worker" | "sharedworker" | "serviceworker" => {
            DevToolsNetworkResourceType::Script
        }
        "track" => DevToolsNetworkResourceType::TextTrack,
        "manifest" => DevToolsNetworkResourceType::Manifest,
        "report" => DevToolsNetworkResourceType::CspViolationReport,
        "" => DevToolsNetworkResourceType::Fetch,
        _ => DevToolsNetworkResourceType::Other,
    }
}

fn exact_shared_worker_target<'a>(
    conn: &'a CdpConnection,
    attachment: &TargetSharedWorkerProtocolAttachmentIdentity,
) -> Option<&'a SharedWorkerTargetState> {
    if !attachment.is_current() {
        return None;
    }
    let target = conn
        .browser_context_by_id(attachment.browser_context_id())?
        .shared_worker_target(attachment.target_id())?;
    (target.renderer_owner_local_host_id == attachment.renderer_owner_local_host_id()
        && target.renderer_instance_id == attachment.renderer_instance_id()
        && target.owner_target_id() == attachment.owner_target_id()
        && target.is_session(attachment.session_id()))
    .then_some(target)
}

fn exact_shared_worker_target_mut<'a>(
    conn: &'a mut CdpConnection,
    attachment: &TargetSharedWorkerProtocolAttachmentIdentity,
) -> Option<&'a mut SharedWorkerTargetState> {
    if !attachment.is_current() {
        return None;
    }
    let target = conn
        .browser_context_by_id_mut(attachment.browser_context_id())?
        .shared_worker_target_mut(attachment.target_id())?;
    (target.renderer_owner_local_host_id == attachment.renderer_owner_local_host_id()
        && target.renderer_instance_id == attachment.renderer_instance_id()
        && target.owner_target_id() == attachment.owner_target_id()
        && target.is_session(attachment.session_id()))
    .then_some(target)
}

fn exact_shared_worker_pending_runtime_console(
    conn: &CdpConnection,
    attachment: &TargetSharedWorkerProtocolAttachmentIdentity,
) -> Option<(Vec<RuntimeConsoleMessageSnapshot>, usize)> {
    let target = exact_shared_worker_target(conn, attachment)?;
    let messages = target
        .pending_runtime_console_messages(attachment.session_id())
        .to_vec();
    (!messages.is_empty()).then(|| (messages, target.console_message_count()))
}

fn mark_exact_shared_worker_console_domain_emitted(
    conn: &mut CdpConnection,
    attachment: &TargetSharedWorkerProtocolAttachmentIdentity,
    console_end: usize,
) {
    if let Some(target) = exact_shared_worker_target_mut(conn, attachment) {
        target.mark_console_domain_emitted(attachment.session_id(), console_end);
    }
}

fn mark_exact_shared_worker_runtime_console_emitted(
    conn: &mut CdpConnection,
    attachment: &TargetSharedWorkerProtocolAttachmentIdentity,
    console_end: usize,
) {
    if let Some(target) = exact_shared_worker_target_mut(conn, attachment) {
        target.mark_runtime_console_emitted(attachment.session_id(), console_end);
    }
}

#[cfg(test)]
#[path = "worker_target_attachment_tests.rs"]
mod worker_target_attachment_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{
        BrowserContext, CdpTargetFilter, CdpTargetFilterEntry, DedicatedWorkerTargetState,
    };

    fn runtime_inspector_messages(messages: Vec<Value>) -> Vec<RendererRuntimeInspectorMessage> {
        messages
            .into_iter()
            .map(RendererRuntimeInspectorMessage::from_v8_inspector_message)
            .collect()
    }

    fn target_info_id(target_info: &DevToolsTargetInfo) -> &str {
        target_info
            .target_id
            .as_ref()
            .expect("target info should include targetId")
            .as_str()
    }

    fn target_info_id_string(target_info: &DevToolsTargetInfo) -> String {
        target_info_id(target_info).to_owned()
    }

    fn destroyed_output_target_id(output: &WorkerTargetLifecycleOutput) -> Option<&str> {
        match output {
            WorkerTargetLifecycleOutput::SharedWorkerDestroyed { target_delta } => {
                Some(target_delta.target_id())
            }
            WorkerTargetLifecycleOutput::ServiceWorkerDestroyed {
                target_delta: Some(target_delta),
                ..
            } => Some(target_delta.target_id()),
            _ => None,
        }
    }

    fn shared_worker_attached_output(
        output: &WorkerTargetLifecycleOutput,
    ) -> Option<(
        &TargetSharedWorkerProtocolAttachmentIdentity,
        &PreparedTargetAttach,
    )> {
        match output {
            WorkerTargetLifecycleOutput::SharedWorkerAttached {
                attachment,
                prepared_attach,
            } => Some((attachment, prepared_attach)),
            _ => None,
        }
    }

    fn shared_worker_detached_output(
        output: &WorkerTargetLifecycleOutput,
    ) -> Option<(
        &TargetSharedWorkerProtocolAttachmentRetirement,
        &TargetSessionDetachCleanupPlan,
    )> {
        match output {
            WorkerTargetLifecycleOutput::SharedWorkerDetached {
                retirement,
                cleanup_plan,
            } => Some((retirement, cleanup_plan)),
            _ => None,
        }
    }

    fn service_worker_attached_output(
        output: &WorkerTargetLifecycleOutput,
    ) -> Option<(
        &TargetServiceWorkerProtocolAttachmentIdentity,
        &PreparedTargetAttach,
    )> {
        match output {
            WorkerTargetLifecycleOutput::ServiceWorkerAttached {
                attachment,
                prepared_attach,
            } => Some((attachment, prepared_attach)),
            _ => None,
        }
    }

    fn shared_worker_info(instance_id: u64) -> RendererSharedWorkerTargetInfo {
        RendererSharedWorkerTargetInfo {
            owner_local_host_id: moli_core::RendererOwnerLocalHostId::new_for_testing(1),
            instance_id: SharedWorkerInstanceId::from_u64(instance_id),
            url: "https://example.test/shared-worker.js".to_owned(),
            name: "worker".to_owned(),
        }
    }

    fn renderer_run() -> RendererServiceWorkerRunIdentity {
        RendererServiceWorkerRunIdentity::fresh()
    }

    fn service_worker_info(version_id: u64) -> RendererServiceWorkerTargetInfo {
        RendererServiceWorkerTargetInfo {
            registration_id: 41,
            version_id,
            script_url: "https://example.test/service-worker.js".to_owned(),
            scope_url: "https://example.test/app/".to_owned(),
            status: RendererServiceWorkerVersionStatus::Installing,
        }
    }

    fn dedicated_worker_fixture() -> (
        CdpConnection,
        TargetPageResidenceIdentity,
        RendererPageResidenceIdentity,
    ) {
        let mut conn = CdpConnection::default();
        let mut context = BrowserContext::new("BID-1".to_owned());
        context.set_active_target_id("TID-page");
        let page_attachment_id = context
            .active_target
            .runtime_slot
            .set_page_attachment_id_for_test(1);
        let owner_page = TargetPageResidenceIdentity::new(
            "BID-1".to_owned(),
            Some("TID-page".to_owned()),
            page_attachment_id,
        );
        conn.browser_context = Some(context);
        let owner_renderer_page = RendererPageResidenceIdentity::new(
            moli_core::RendererOwnerLocalHostId::new_for_testing(17),
            moli_core::PageId::new_for_testing(23),
        );
        (conn, owner_page, owner_renderer_page)
    }

    fn enable_dedicated_worker_auto_attach_for_owner_page(
        conn: &mut CdpConnection,
        wait_for_debugger_on_start: bool,
    ) {
        assert!(conn.prepare_auto_attached_page_session_binding(
            "TID-page",
            "SID-page-base".to_owned(),
        ));
        conn.set_auto_attach_owner(
            Some("SID-page-base"),
            true,
            wait_for_debugger_on_start,
            CdpTargetFilter::default_auto_attach(),
        );
    }

    fn dedicated_worker_info(
        instance_id: u64,
        owner_renderer_page: RendererPageResidenceIdentity,
        request_url: &str,
    ) -> RendererDedicatedWorkerTargetInfo {
        RendererDedicatedWorkerTargetInfo {
            owner_local_host_id: owner_renderer_page.owner_local_host_id(),
            page_id: owner_renderer_page.page_id(),
            instance_id,
            request_url: request_url.to_owned(),
            document_url: "https://example.test/page.html".to_owned(),
            name: String::new(),
        }
    }

    fn protocol_messages(events: &[BackgroundProtocolEvent]) -> Vec<Value> {
        events
            .iter()
            .cloned()
            .map(BackgroundProtocolEvent::into_protocol_message)
            .collect()
    }

    fn dedicated_output_protocol_messages(outputs: &TargetPreparedOutputs) -> Vec<Value> {
        outputs
            .worker_target_lifecycle_outputs
            .iter()
            .filter_map(|output| match output {
                WorkerTargetLifecycleOutput::DedicatedWorkerEvents { events, .. } => Some(events),
                _ => None,
            })
            .flat_map(|events| protocol_messages(events))
            .collect()
    }

    fn dedicated_created_target_id(outputs: &TargetPreparedOutputs) -> String {
        outputs
            .worker_target_lifecycle_outputs
            .iter()
            .find_map(|output| match output {
                WorkerTargetLifecycleOutput::DedicatedWorkerCreated { target_delta, .. } => {
                    Some(target_delta.target_id().to_owned())
                }
                _ => None,
            })
            .expect("dedicated worker targetCreated output")
    }

    fn dedicated_attached_session_id(outputs: &TargetPreparedOutputs) -> String {
        outputs
            .worker_target_lifecycle_outputs
            .iter()
            .find_map(|output| match output {
                WorkerTargetLifecycleOutput::DedicatedWorkerAttached { session_id, .. } => {
                    Some(session_id.clone())
                }
                _ => None,
            })
            .expect("dedicated worker attached output")
    }

    fn worker_response(
        url: &str,
        status: u16,
        transport: bool,
    ) -> moli_core::page::NavigationResponse {
        let mut response = moli_core::page::NavigationResponse::from_text_body(
            Url::parse(url).unwrap(),
            status,
            vec![("content-type".to_owned(), "text/javascript".to_owned())],
            "postMessage('ready')".to_owned(),
        );
        if transport {
            response = response.with_network_request_headers(Some(vec![(
                "User-Agent".to_owned(),
                "fixture".to_owned(),
            )]));
            response.negotiated_http_version = Some(moli_fetch::NegotiatedHttpVersion::Http2);
        }
        response
    }

    fn worker_context_created_event(
        context_id: i64,
        context_type: &str,
    ) -> RuntimeExecutionContextEvent {
        RuntimeExecutionContextEvent {
            target_id: None,
            context_id: Some(context_id),
            realm_id: None,
            frame_id: None,
            origin: None,
            name: None,
            is_default: None,
            context_type: Some(context_type.to_owned()),
            grant_universal_access: None,
        }
    }

    fn target_filter_excluding(target_type: &str) -> CdpTargetFilter {
        CdpTargetFilter::from_entries(vec![
            CdpTargetFilterEntry {
                exclude: true,
                target_type: Some(target_type.to_owned()),
            },
            CdpTargetFilterEntry {
                exclude: false,
                target_type: Some("page".to_owned()),
            },
        ])
    }

    fn lifecycle_protocol_message(
        outputs: &[WorkerTargetLifecycleOutput],
        method: &str,
    ) -> Option<Value> {
        outputs.iter().find_map(|output| match output {
            WorkerTargetLifecycleOutput::ServiceWorkerVersionEvents { events, .. }
            | WorkerTargetLifecycleOutput::ServiceWorkerAttachmentEvents { events, .. }
            | WorkerTargetLifecycleOutput::ServiceWorkerRunEvents { events, .. }
            | WorkerTargetLifecycleOutput::ServiceWorkerRuntimeEvents { events, .. } => events
                .iter()
                .cloned()
                .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
                .find(|message| message["method"].as_str() == Some(method)),
            _ => None,
        })
    }

    async fn drain_target_lifecycle_events_for_test(
        conn: &mut CdpConnection,
        outputs: TargetPreparedOutputs,
    ) -> Vec<crate::conn::BackgroundProtocolEvent> {
        let mut prepared_outputs =
            ProtocolOutputPayloads::from_slot(TargetPreparedOutputSlot::from_outputs(outputs));
        let mut command_context = crate::conn::CommandDispatchContext::default();
        emit_target_lifecycle_events(
            conn,
            &mut ProtocolOutputProjectionContext::new(None, &mut command_context),
            Some(&mut prepared_outputs),
        )
        .await;
        command_context.take_protocol_events()
    }

    #[tokio::test]
    async fn dedicated_worker_creation_precedes_page_main_script_request_and_uses_target_id() {
        let (mut conn, owner_page, owner_renderer_page) = dedicated_worker_fixture();
        conn.set_target_discovery_for_owner(None, CdpTargetFilter::default_target_discovery());

        let outputs = register_dedicated_worker_target(
            &mut conn,
            "BID-1",
            owner_page,
            owner_renderer_page,
            vec![None],
            dedicated_worker_info(7, owner_renderer_page, "https://example.test/worker.js"),
        );
        assert!(matches!(
            outputs.worker_target_lifecycle_outputs.first(),
            Some(WorkerTargetLifecycleOutput::DedicatedWorkerCreated { .. })
        ));
        assert!(matches!(
            outputs.worker_target_lifecycle_outputs.get(1),
            Some(WorkerTargetLifecycleOutput::DedicatedWorkerEvents { .. })
        ));
        let target_id = dedicated_created_target_id(&outputs);
        let target_info = conn
            .browser_context
            .as_ref()
            .unwrap()
            .devtools_target_info(&target_id)
            .expect("dedicated worker target info");
        assert_eq!(target_info.kind, DevToolsTargetKind::Worker);
        assert_eq!(target_info.url, "");
        assert_eq!(
            target_info.opener_id.as_ref().map(|id| id.as_str()),
            Some("TID-page")
        );

        let events = drain_target_lifecycle_events_for_test(&mut conn, outputs).await;
        let messages = protocol_messages(&events);
        assert_eq!(
            messages
                .iter()
                .filter_map(|message| message["method"].as_str())
                .collect::<Vec<_>>(),
            &["Target.targetCreated", "Network.requestWillBeSent"]
        );
        assert_eq!(messages[0]["params"]["targetInfo"]["targetId"], target_id);
        assert_eq!(messages[0]["params"]["targetInfo"]["title"], "");
        assert_eq!(messages[0]["params"]["targetInfo"]["url"], "");
        assert_eq!(messages[0]["params"]["targetInfo"]["attached"], false);
        assert_eq!(messages[0]["params"]["targetInfo"]["type"], "worker");
        assert_eq!(messages[1]["params"]["requestId"], target_id);
        assert_eq!(messages[1]["params"]["loaderId"], "TID-page");
        assert_eq!(messages[1]["params"]["frameId"], "TID-page");
        assert_eq!(messages[1]["params"]["type"], "Script");
        assert_eq!(messages[1]["params"]["initiator"]["type"], "other");
        assert_eq!(
            messages[1]["params"]["request"]["url"],
            "https://example.test/worker.js"
        );
    }

    #[test]
    fn dedicated_worker_creation_rejects_a_renderer_page_identity_mismatch() {
        let (mut conn, owner_page, owner_renderer_page) = dedicated_worker_fixture();
        let mut info =
            dedicated_worker_info(8, owner_renderer_page, "https://example.test/worker.js");
        info.page_id = moli_core::PageId::new_for_testing(24);

        assert!(
            register_dedicated_worker_target(
                &mut conn,
                "BID-1",
                owner_page,
                owner_renderer_page,
                vec![None],
                info,
            )
            .is_empty()
        );
        assert!(
            conn.browser_context
                .as_ref()
                .unwrap()
                .dedicated_worker_targets
                .is_empty()
        );
    }

    #[tokio::test]
    async fn dedicated_worker_auto_attach_is_scoped_to_its_owner_page_session() {
        let (mut conn, owner_page, owner_renderer_page) = dedicated_worker_fixture();
        assert!(conn.prepare_auto_attached_page_session_binding(
            "TID-page",
            "SID-page-base".to_owned(),
        ));
        conn.browser_context
            .as_mut()
            .unwrap()
            .background_targets
            .push(crate::conn::BackgroundTarget::with_url(
                "TID-other-page".to_owned(),
                Some("SID-other-page".to_owned()),
                "about:blank".to_owned(),
            ));
        for owner in [None, Some("SID-page-base"), Some("SID-other-page")] {
            conn.set_auto_attach_owner(owner, true, false, CdpTargetFilter::default_auto_attach());
        }

        let created = register_dedicated_worker_target(
            &mut conn,
            "BID-1",
            owner_page,
            owner_renderer_page,
            vec![Some("SID-page-base".to_owned())],
            dedicated_worker_info(81, owner_renderer_page, "https://example.test/worker.js"),
        );
        let _ = drain_target_lifecycle_events_for_test(&mut conn, created).await;
        let loaded = record_dedicated_worker_main_script(
            &mut conn,
            "BID-1",
            81,
            "https://example.test/worker.js".to_owned(),
            crate::conn::DedicatedWorkerMainScriptOutcome::Loaded(Box::new(worker_response(
                "https://example.test/worker.js",
                200,
                true,
            ))),
        );

        assert_eq!(
            loaded
                .worker_target_lifecycle_outputs
                .iter()
                .filter(|output| matches!(
                    output,
                    WorkerTargetLifecycleOutput::DedicatedWorkerAttached { .. }
                ))
                .count(),
            1,
            "browser/root and unrelated Page owners must not receive this worker"
        );
        let messages =
            protocol_messages(&drain_target_lifecycle_events_for_test(&mut conn, loaded).await);
        let attached = messages
            .iter()
            .find(|message| message["method"] == "Target.attachedToTarget")
            .expect("owner Page attachment");
        assert_eq!(attached["sessionId"], "SID-page-base");
    }

    #[tokio::test]
    async fn dedicated_worker_paused_http_main_script_splits_page_extra_info_from_worker_completion()
     {
        let (mut conn, owner_page, owner_renderer_page) = dedicated_worker_fixture();
        conn.set_target_discovery_for_owner(None, CdpTargetFilter::default_target_discovery());
        enable_dedicated_worker_auto_attach_for_owner_page(&mut conn, true);
        let created = register_dedicated_worker_target(
            &mut conn,
            "BID-1",
            owner_page,
            owner_renderer_page,
            vec![None],
            dedicated_worker_info(9, owner_renderer_page, "https://example.test/worker.js"),
        );
        let target_id = dedicated_created_target_id(&created);
        let _ = drain_target_lifecycle_events_for_test(&mut conn, created).await;

        let loaded = record_dedicated_worker_main_script(
            &mut conn,
            "BID-1",
            9,
            "https://example.test/worker.js".to_owned(),
            crate::conn::DedicatedWorkerMainScriptOutcome::Loaded(Box::new(worker_response(
                "https://example.test/worker.js",
                200,
                true,
            ))),
        );
        let page_messages = dedicated_output_protocol_messages(&loaded);
        assert_eq!(
            page_messages
                .iter()
                .filter_map(|message| message["method"].as_str())
                .collect::<Vec<_>>(),
            &[
                "Network.requestWillBeSentExtraInfo",
                "Network.responseReceivedExtraInfo"
            ]
        );
        assert!(page_messages.iter().all(|message| {
            message["method"] != "Network.responseReceived"
                && message["method"] != "Network.loadingFinished"
        }));
        let session_id = dedicated_attached_session_id(&loaded);
        let lifecycle_events = drain_target_lifecycle_events_for_test(&mut conn, loaded).await;
        let lifecycle_messages = protocol_messages(&lifecycle_events);
        let lifecycle_methods = lifecycle_messages
            .iter()
            .filter_map(|message| message["method"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            lifecycle_methods,
            &[
                "Network.requestWillBeSentExtraInfo",
                "Network.responseReceivedExtraInfo",
                "Target.targetInfoChanged",
                "Target.attachedToTarget"
            ]
        );
        assert_eq!(
            lifecycle_messages
                .iter()
                .find(|message| message["method"] == "Target.targetInfoChanged")
                .unwrap()["params"]["targetInfo"]["url"],
            "https://example.test/worker.js"
        );
        assert_eq!(
            lifecycle_messages
                .iter()
                .find(|message| message["method"] == "Target.targetInfoChanged")
                .unwrap()["params"]["targetInfo"]["attached"],
            true
        );
        assert_eq!(
            conn.session_route(Some(&session_id)),
            Some(crate::conn::CdpSessionRoute::DedicatedWorkerTarget {
                browser_context_id: "BID-1".to_owned(),
                target_id: target_id.clone(),
            })
        );

        assert!(conn.enable_network_listener_for_session_owner(Some(&session_id)));
        let worker_events =
            dedicated_worker_main_script_network_replay_for_session(&mut conn, &session_id);
        let worker_messages = protocol_messages(&worker_events);
        assert_eq!(
            worker_messages
                .iter()
                .filter_map(|message| message["method"].as_str())
                .collect::<Vec<_>>(),
            &["Network.responseReceived", "Network.loadingFinished"]
        );
        assert!(worker_messages.iter().all(|message| {
            message["method"] != "Network.requestWillBeSent"
                && message["method"] != "Network.responseReceivedExtraInfo"
        }));
        assert_eq!(worker_messages[0]["sessionId"], session_id);
        assert_eq!(worker_messages[0]["params"]["requestId"], target_id);
        assert_eq!(worker_messages[0]["params"]["type"], "Script");
        assert_eq!(worker_messages[0]["params"]["hasExtraInfo"], true);
        assert_eq!(worker_messages[0]["params"]["response"]["protocol"], "h2");
        assert_eq!(worker_messages[1]["params"]["requestId"], target_id);
        assert!(
            dedicated_worker_main_script_network_replay_for_session(&mut conn, &session_id)
                .is_empty(),
            "Network.enable/replay must not duplicate the worker main-script completion"
        );
    }

    #[tokio::test]
    async fn dedicated_worker_non_paused_auto_attach_does_not_replay_completed_main_script() {
        let (mut conn, owner_page, owner_renderer_page) = dedicated_worker_fixture();
        enable_dedicated_worker_auto_attach_for_owner_page(&mut conn, false);
        let created = register_dedicated_worker_target(
            &mut conn,
            "BID-1",
            owner_page,
            owner_renderer_page,
            vec![None],
            dedicated_worker_info(90, owner_renderer_page, "https://example.test/worker.js"),
        );
        let _ = drain_target_lifecycle_events_for_test(&mut conn, created).await;
        let loaded = record_dedicated_worker_main_script(
            &mut conn,
            "BID-1",
            90,
            "https://example.test/worker.js".to_owned(),
            crate::conn::DedicatedWorkerMainScriptOutcome::Loaded(Box::new(worker_response(
                "https://example.test/worker.js",
                200,
                true,
            ))),
        );
        let session_id = dedicated_attached_session_id(&loaded);
        let _ = drain_target_lifecycle_events_for_test(&mut conn, loaded).await;

        assert!(conn.enable_network_listener_for_session_owner(Some(&session_id)));
        assert!(
            dedicated_worker_main_script_network_replay_for_session(&mut conn, &session_id)
                .is_empty(),
            "Chromium does not replay a main-script response to an auto-attached worker that was not paused"
        );
    }

    #[tokio::test]
    async fn dedicated_worker_debugger_resume_discards_unobserved_main_script_completion() {
        let (mut conn, owner_page, owner_renderer_page) = dedicated_worker_fixture();
        enable_dedicated_worker_auto_attach_for_owner_page(&mut conn, true);
        let created = register_dedicated_worker_target(
            &mut conn,
            "BID-1",
            owner_page,
            owner_renderer_page,
            vec![None],
            dedicated_worker_info(91, owner_renderer_page, "https://example.test/worker.js"),
        );
        let _ = drain_target_lifecycle_events_for_test(&mut conn, created).await;
        let loaded = record_dedicated_worker_main_script(
            &mut conn,
            "BID-1",
            91,
            "https://example.test/worker.js".to_owned(),
            crate::conn::DedicatedWorkerMainScriptOutcome::Loaded(Box::new(worker_response(
                "https://example.test/worker.js",
                200,
                true,
            ))),
        );
        let session_id = dedicated_attached_session_id(&loaded);
        let _ = drain_target_lifecycle_events_for_test(&mut conn, loaded).await;

        assert_eq!(
            conn.run_dedicated_worker_if_waiting_for_debugger_for_session(Some(&session_id)),
            Ok(false),
            "the unit fixture has no live renderer worker, but the resume boundary must still consume replay eligibility"
        );
        assert!(conn.enable_network_listener_for_session_owner(Some(&session_id)));
        assert!(
            dedicated_worker_main_script_network_replay_for_session(&mut conn, &session_id)
                .is_empty(),
            "Network.enable after debugger resume must not recover historical completion events"
        );
    }

    #[test]
    fn dedicated_worker_direct_attach_after_load_does_not_replay_main_script_completion() {
        let (mut conn, owner_page, owner_renderer_page) = dedicated_worker_fixture();
        let _ = register_dedicated_worker_target(
            &mut conn,
            "BID-1",
            owner_page,
            owner_renderer_page,
            Vec::new(),
            dedicated_worker_info(92, owner_renderer_page, "https://example.test/worker.js"),
        );
        let target_id = conn
            .browser_context
            .as_ref()
            .unwrap()
            .dedicated_worker_target_id_for_renderer_instance(92)
            .unwrap()
            .to_owned();
        let completed = record_dedicated_worker_main_script(
            &mut conn,
            "BID-1",
            92,
            "https://example.test/worker.js".to_owned(),
            crate::conn::DedicatedWorkerMainScriptOutcome::Loaded(Box::new(worker_response(
                "https://example.test/worker.js",
                200,
                true,
            ))),
        );
        assert!(
            completed
                .worker_target_lifecycle_outputs
                .iter()
                .all(|output| !matches!(
                    output,
                    WorkerTargetLifecycleOutput::DedicatedWorkerAttached { .. }
                ))
        );
        conn.attach_dedicated_worker_target_session_event_plan(
            "SID-direct-worker".to_owned(),
            None,
            &target_id,
        )
        .expect("direct DedicatedWorker attachment");

        assert!(conn.enable_network_listener_for_session_owner(Some("SID-direct-worker")));
        assert!(
            dedicated_worker_main_script_network_replay_for_session(&mut conn, "SID-direct-worker")
                .is_empty(),
            "a manual attachment made after load must not receive historical completion events"
        );
    }

    #[tokio::test]
    async fn dedicated_worker_blob_and_failed_main_scripts_publish_chromium_terminal_shapes() {
        for (instance_id, url, outcome, terminal_method, expected_protocol, has_extra_info) in
            [
                (
                    10,
                    "blob:https://example.test/worker-blob",
                    crate::conn::DedicatedWorkerMainScriptOutcome::Loaded(Box::new(
                        worker_response("blob:https://example.test/worker-blob", 200, false),
                    )),
                    "Network.loadingFinished",
                    Some("blob"),
                    false,
                ),
                (
                    11,
                    "https://example.test/missing-worker.js",
                    crate::conn::DedicatedWorkerMainScriptOutcome::Failed {
                        error_message: "load failed: net::ERR_FAILED".to_owned(),
                        response: Some(Box::new(worker_response(
                            "https://example.test/missing-worker.js",
                            404,
                            true,
                        ))),
                    },
                    "Network.loadingFailed",
                    Some("h2"),
                    true,
                ),
            ]
        {
            let (mut conn, owner_page, owner_renderer_page) = dedicated_worker_fixture();
            enable_dedicated_worker_auto_attach_for_owner_page(&mut conn, true);
            let created = register_dedicated_worker_target(
                &mut conn,
                "BID-1",
                owner_page,
                owner_renderer_page,
                vec![None],
                dedicated_worker_info(instance_id, owner_renderer_page, url),
            );
            let target_id = conn
                .browser_context
                .as_ref()
                .unwrap()
                .dedicated_worker_target_id_for_renderer_instance(instance_id)
                .unwrap()
                .to_owned();
            let _ = drain_target_lifecycle_events_for_test(&mut conn, created).await;
            let completed = record_dedicated_worker_main_script(
                &mut conn,
                "BID-1",
                instance_id,
                url.to_owned(),
                outcome,
            );
            let session_id = dedicated_attached_session_id(&completed);
            let _ = drain_target_lifecycle_events_for_test(&mut conn, completed).await;
            assert!(conn.enable_network_listener_for_session_owner(Some(&session_id)));
            let messages = protocol_messages(
                &dedicated_worker_main_script_network_replay_for_session(&mut conn, &session_id),
            );
            assert_eq!(messages[0]["method"], "Network.responseReceived");
            assert_eq!(messages[0]["params"]["requestId"], target_id);
            assert_eq!(messages[0]["params"]["hasExtraInfo"], has_extra_info);
            assert_eq!(
                messages[0]["params"]["response"]["protocol"].as_str(),
                expected_protocol
            );
            assert_eq!(messages[1]["method"], terminal_method);
            if terminal_method == "Network.loadingFailed" {
                assert_eq!(messages[1]["params"]["errorText"], "net::ERR_FAILED");
                assert_eq!(messages[1]["params"]["canceled"], false);
            }
        }
    }

    #[tokio::test]
    async fn dedicated_worker_failed_load_created_and_destroyed_in_one_capture_is_not_filtered() {
        let (mut conn, owner_page, owner_renderer_page) = dedicated_worker_fixture();
        conn.set_target_discovery_for_owner(None, CdpTargetFilter::default_target_discovery());
        enable_dedicated_worker_auto_attach_for_owner_page(&mut conn, false);
        let script_url = "https://example.test/missing-worker.js";
        let outputs = dedicated_worker_target_lifecycle_outputs_for_events(
            &mut conn,
            "BID-1".to_owned(),
            owner_page,
            owner_renderer_page,
            vec![None],
            vec![
                RendererDedicatedWorkerTargetEvent::Created(dedicated_worker_info(
                    13,
                    owner_renderer_page,
                    script_url,
                )),
                RendererDedicatedWorkerTargetEvent::ScriptLoadFailed {
                    instance_id: 13,
                    script_url: script_url.to_owned(),
                    error_message: "load failed: net::ERR_FAILED".to_owned(),
                    response: Some(Box::new(worker_response(script_url, 404, true))),
                },
                RendererDedicatedWorkerTargetEvent::Destroyed { instance_id: 13 },
            ],
        );
        let target_id = dedicated_created_target_id(&outputs);
        assert_eq!(
            conn.target_registry_host_kind(&target_id),
            Some(DevToolsTargetKind::Worker),
            "preparation must retain the target until earlier outputs are projected"
        );

        let messages =
            protocol_messages(&drain_target_lifecycle_events_for_test(&mut conn, outputs).await);
        assert_eq!(
            messages
                .iter()
                .filter_map(|message| message["method"].as_str())
                .collect::<Vec<_>>(),
            &[
                "Target.targetCreated",
                "Network.requestWillBeSent",
                "Network.requestWillBeSentExtraInfo",
                "Network.responseReceivedExtraInfo",
                "Target.targetInfoChanged",
                "Target.attachedToTarget",
                "Target.targetInfoChanged",
                "Target.detachedFromTarget",
                "Target.targetDestroyed",
            ]
        );
        assert_eq!(messages[0]["params"]["targetInfo"]["url"], "");
        assert_eq!(messages[1]["params"]["requestId"], target_id);
        assert_eq!(messages[3]["params"]["statusCode"], 404);
        assert_eq!(messages[4]["params"]["targetInfo"]["url"], script_url);
        assert_eq!(messages[6]["params"]["targetInfo"]["attached"], false);
        assert_eq!(messages[8]["params"]["targetId"], target_id);
        assert_eq!(conn.target_registry_host_kind(&target_id), None);
    }

    #[tokio::test]
    async fn dedicated_worker_failed_load_waits_for_debugger_before_target_retirement() {
        let (mut conn, owner_page, owner_renderer_page) = dedicated_worker_fixture();
        assert!(conn.prepare_auto_attached_page_session_binding(
            "TID-page",
            "SID-page-base".to_owned(),
        ));
        conn.set_target_discovery_for_owner(
            Some("SID-page-base"),
            CdpTargetFilter::default_target_discovery(),
        );
        conn.set_auto_attach_owner(
            Some("SID-page-base"),
            true,
            true,
            CdpTargetFilter::default_auto_attach(),
        );
        let script_url = "https://example.test/missing-worker.js";
        let outputs = dedicated_worker_target_lifecycle_outputs_for_events(
            &mut conn,
            "BID-1".to_owned(),
            owner_page,
            owner_renderer_page,
            vec![None],
            vec![
                RendererDedicatedWorkerTargetEvent::Created(dedicated_worker_info(
                    14,
                    owner_renderer_page,
                    script_url,
                )),
                RendererDedicatedWorkerTargetEvent::ScriptLoadFailed {
                    instance_id: 14,
                    script_url: script_url.to_owned(),
                    error_message: "load failed: net::ERR_FAILED".to_owned(),
                    response: Some(Box::new(worker_response(script_url, 404, true))),
                },
                RendererDedicatedWorkerTargetEvent::Destroyed { instance_id: 14 },
            ],
        );
        let target_id = dedicated_created_target_id(&outputs);
        let session_id = dedicated_attached_session_id(&outputs);
        let messages =
            protocol_messages(&drain_target_lifecycle_events_for_test(&mut conn, outputs).await);
        assert_eq!(
            messages
                .iter()
                .filter_map(|message| message["method"].as_str())
                .collect::<Vec<_>>(),
            &[
                "Target.targetCreated",
                "Network.requestWillBeSent",
                "Network.requestWillBeSentExtraInfo",
                "Network.responseReceivedExtraInfo",
                "Target.targetInfoChanged",
                "Target.attachedToTarget",
            ]
        );
        assert_eq!(
            conn.target_registry_host_kind(&target_id),
            Some(DevToolsTargetKind::Worker)
        );

        conn.dedicated_worker_target_for_session_mut(Some(&session_id))
            .expect("failed worker target session")
            .set_runtime_frontend_enabled(&session_id, true);

        assert!(conn.enable_network_listener_for_session_owner(Some(&session_id)));
        let worker_messages = protocol_messages(
            &dedicated_worker_main_script_network_replay_for_session(&mut conn, &session_id),
        );
        assert_eq!(
            worker_messages
                .iter()
                .filter_map(|message| message["method"].as_str())
                .collect::<Vec<_>>(),
            &["Network.responseReceived", "Network.loadingFailed"]
        );
        assert_eq!(worker_messages[0]["params"]["response"]["status"], 404);
        assert_eq!(worker_messages[1]["params"]["errorText"], "net::ERR_FAILED");

        let run_params = json!({});
        let run_json = json!({
            "id": 71,
            "method": "Runtime.runIfWaitingForDebugger",
            "sessionId": session_id,
            "params": run_params,
        })
        .to_string();
        let run_cmd = crate::conn::Cmd::for_test(
            Some(71),
            "Runtime.runIfWaitingForDebugger",
            &run_params,
            Some(&session_id),
            &run_json,
        );
        let Some(crate::domains::runtime::RuntimeCommandTaskStep::Complete(run_plan)) =
            crate::domains::runtime::try_start_runtime_command_dispatch(&mut conn, &run_cmd)
        else {
            panic!("failed DedicatedWorker debugger resume should complete synchronously");
        };
        let retirement_messages =
            protocol_messages(&run_plan.into_background_events(run_cmd.id, run_cmd.session_id));
        assert_eq!(retirement_messages[0]["result"], json!({}));
        assert_eq!(
            retirement_messages
                .iter()
                .filter_map(|message| message["method"].as_str())
                .collect::<Vec<_>>(),
            &[
                "Target.targetInfoChanged",
                "Target.detachedFromTarget",
                "Target.targetDestroyed"
            ]
        );
        assert_eq!(
            retirement_messages
                .iter()
                .find(|message| message["method"] == "Target.detachedFromTarget")
                .expect("detached event")["sessionId"],
            "SID-page-base"
        );
        assert_eq!(conn.session_route(Some(&session_id)), None);
        assert_eq!(conn.target_registry_host_kind(&target_id), None);
    }

    #[tokio::test]
    async fn dedicated_worker_destruction_marks_detached_before_session_and_target_retirement() {
        let (mut conn, owner_page, owner_renderer_page) = dedicated_worker_fixture();
        conn.set_target_discovery_for_owner(None, CdpTargetFilter::default_target_discovery());
        enable_dedicated_worker_auto_attach_for_owner_page(&mut conn, false);
        let created = register_dedicated_worker_target(
            &mut conn,
            "BID-1",
            owner_page,
            owner_renderer_page,
            vec![None],
            dedicated_worker_info(
                12,
                owner_renderer_page,
                "blob:https://example.test/worker-blob",
            ),
        );
        let target_id = dedicated_created_target_id(&created);
        let _ = drain_target_lifecycle_events_for_test(&mut conn, created).await;
        let loaded = record_dedicated_worker_main_script(
            &mut conn,
            "BID-1",
            12,
            "blob:https://example.test/worker-blob".to_owned(),
            crate::conn::DedicatedWorkerMainScriptOutcome::Loaded(Box::new(worker_response(
                "blob:https://example.test/worker-blob",
                200,
                false,
            ))),
        );
        let session_id = dedicated_attached_session_id(&loaded);
        let _ = drain_target_lifecycle_events_for_test(&mut conn, loaded).await;

        let removed = prepare_dedicated_worker_target_retirement(
            &mut conn,
            "BID-1",
            12,
            DedicatedWorkerRetirementCause::RendererDestroyed,
        );
        let messages =
            protocol_messages(&drain_target_lifecycle_events_for_test(&mut conn, removed).await);
        assert_eq!(
            messages
                .iter()
                .filter_map(|message| message["method"].as_str())
                .collect::<Vec<_>>(),
            &[
                "Target.targetInfoChanged",
                "Target.detachedFromTarget",
                "Target.targetDestroyed"
            ]
        );
        assert_eq!(messages[0]["params"]["targetInfo"]["targetId"], target_id);
        assert_eq!(messages[0]["params"]["targetInfo"]["attached"], false);
        assert_eq!(messages[1]["params"]["sessionId"], session_id);
        assert_eq!(messages[1]["params"]["targetId"], target_id);
        assert_eq!(messages[2]["params"]["targetId"], target_id);
        assert_eq!(conn.session_route(Some(&session_id)), None);
        assert_eq!(conn.target_registry_host_kind(&target_id), None);
    }

    #[tokio::test]
    async fn dedicated_worker_destruction_retires_state_without_target_discovery() {
        let (mut conn, owner_page, owner_renderer_page) = dedicated_worker_fixture();
        let outputs = register_dedicated_worker_target(
            &mut conn,
            "BID-1",
            owner_page,
            owner_renderer_page,
            Vec::new(),
            dedicated_worker_info(13, owner_renderer_page, "https://example.test/worker.js"),
        );
        assert!(outputs.worker_target_lifecycle_outputs.is_empty());
        let target_id = conn
            .browser_context
            .as_ref()
            .and_then(|context| context.dedicated_worker_target_id_for_renderer_instance(13))
            .expect("DedicatedWorker target state")
            .to_owned();
        assert_eq!(
            conn.target_registry_host_kind(&target_id),
            Some(DevToolsTargetKind::Worker)
        );

        let retirement = prepare_dedicated_worker_target_retirement(
            &mut conn,
            "BID-1",
            13,
            DedicatedWorkerRetirementCause::RendererDestroyed,
        );
        let events = drain_target_lifecycle_events_for_test(&mut conn, retirement).await;

        assert!(events.is_empty(), "disabled discovery must remain silent");
        assert!(
            conn.browser_context
                .as_ref()
                .unwrap()
                .dedicated_worker_targets
                .is_empty(),
            "protocol state retirement must not depend on discovery visibility"
        );
        assert_eq!(conn.target_registry_host_kind(&target_id), None);
    }

    #[tokio::test]
    async fn page_replacement_retires_only_its_owned_dedicated_workers() {
        let (mut conn, owner_page, owner_renderer_page) = dedicated_worker_fixture();
        conn.set_target_discovery_for_owner(None, CdpTargetFilter::default_target_discovery());
        enable_dedicated_worker_auto_attach_for_owner_page(&mut conn, false);
        let created = register_dedicated_worker_target(
            &mut conn,
            "BID-1",
            owner_page.clone(),
            owner_renderer_page,
            vec![Some("SID-page-base".to_owned())],
            dedicated_worker_info(14, owner_renderer_page, "https://example.test/worker.js"),
        );
        let retired_target_id = dedicated_created_target_id(&created);
        let _ = drain_target_lifecycle_events_for_test(&mut conn, created).await;
        let loaded = record_dedicated_worker_main_script(
            &mut conn,
            "BID-1",
            14,
            "https://example.test/worker.js".to_owned(),
            crate::conn::DedicatedWorkerMainScriptOutcome::Loaded(Box::new(worker_response(
                "https://example.test/worker.js",
                200,
                true,
            ))),
        );
        let retired_session_id = dedicated_attached_session_id(&loaded);
        let _ = drain_target_lifecycle_events_for_test(&mut conn, loaded).await;

        let retained_target_id = "TID-other-worker".to_owned();
        let retained_owner = TargetPageResidenceIdentity::new_for_test(
            "BID-1".to_owned(),
            Some("TID-other-page".to_owned()),
            7,
        );
        conn.browser_context
            .as_mut()
            .unwrap()
            .insert_dedicated_worker_target(crate::conn::DedicatedWorkerTargetState::new(
                retained_owner,
                owner_renderer_page.owner_local_host_id(),
                15,
                retained_target_id.clone(),
                String::new(),
                Vec::new(),
            ));
        conn.register_worker_target_host(&retained_target_id, DevToolsTargetKind::Worker);

        conn.browser_context
            .as_mut()
            .unwrap()
            .active_target
            .runtime_slot
            .replace_page_attachment_id_for_test();
        let messages = protocol_messages(
            &retire_dedicated_worker_targets_for_replaced_page_async(&mut conn, &owner_page).await,
        );

        assert_eq!(
            messages
                .iter()
                .filter_map(|message| message["method"].as_str())
                .collect::<Vec<_>>(),
            &[
                "Target.targetInfoChanged",
                "Target.detachedFromTarget",
                "Target.targetDestroyed"
            ]
        );
        assert_eq!(messages[0]["params"]["targetInfo"]["attached"], false);
        assert_eq!(messages[1]["params"]["sessionId"], retired_session_id);
        assert_eq!(messages[2]["params"]["targetId"], retired_target_id);
        assert_eq!(conn.session_route(Some(&retired_session_id)), None);
        assert_eq!(conn.target_registry_host_kind(&retired_target_id), None);
        assert!(
            !conn
                .browser_context
                .as_ref()
                .unwrap()
                .dedicated_worker_targets
                .contains_key(&14)
        );
        assert!(
            conn.browser_context
                .as_ref()
                .unwrap()
                .dedicated_worker_targets
                .contains_key(&15),
            "a parallel Page residence must retain its worker"
        );
        assert_eq!(
            conn.target_registry_host_kind(&retained_target_id),
            Some(DevToolsTargetKind::Worker)
        );
    }

    #[test]
    fn shared_worker_target_registration_updates_browser_context_and_emits_when_discovered() {
        let mut conn = CdpConnection::default();
        conn.set_root_target_discovery_enabled(true);
        conn.browser_context = Some(BrowserContext::new("BID-1".to_owned()));

        let mut outputs =
            register_shared_worker_target(&mut conn, "BID-1", None, shared_worker_info(7))
                .worker_target_lifecycle_outputs
                .into_iter();
        let output = outputs
            .next()
            .expect("discovered shared worker should emit targetCreated");
        let WorkerTargetLifecycleOutput::SharedWorkerCreated { target_delta } = output else {
            panic!("expected targetCreated output");
        };
        let target_id = target_delta.target_id().to_owned();
        assert!(outputs.next().is_none());
        let target_info = conn
            .browser_context
            .as_ref()
            .unwrap()
            .devtools_target_info(&target_id)
            .expect("created shared worker target info");
        assert_eq!(target_info.kind, DevToolsTargetKind::SharedWorker);
        assert_eq!(target_info.title, "worker");
        assert_eq!(
            conn.target_registry_host_kind(&target_id),
            Some(DevToolsTargetKind::SharedWorker)
        );

        let context = conn.browser_context.as_ref().unwrap();
        assert_eq!(
            context
                .target_info(&target_id)
                .expect("shared worker target should be addressable")["url"],
            "https://example.test/shared-worker.js"
        );
        assert!(
            register_shared_worker_target(&mut conn, "BID-1", None, shared_worker_info(7))
                .is_empty(),
            "duplicate created events for the same renderer instance should not allocate a second target"
        );

        let mut outputs =
            remove_shared_worker_target(&mut conn, "BID-1", SharedWorkerInstanceId::from_u64(7))
                .worker_target_lifecycle_outputs
                .into_iter();
        let output = outputs
            .next()
            .expect("discovered shared worker should emit targetDestroyed");
        assert_eq!(
            destroyed_output_target_id(&output),
            Some(target_id.as_str())
        );
        assert!(outputs.next().is_none());
        assert_eq!(conn.target_registry_host_kind(&target_id), None);
        assert!(
            conn.browser_context
                .as_ref()
                .unwrap()
                .target_info(&target_id)
                .is_none()
        );
    }

    #[test]
    fn shared_worker_target_registration_tracks_undiscovered_targets_without_events() {
        let mut conn = CdpConnection::default();
        conn.browser_context = Some(BrowserContext::new("BID-1".to_owned()));

        assert!(
            register_shared_worker_target(&mut conn, "BID-1", None, shared_worker_info(9))
                .is_empty(),
            "target discovery disabled should suppress immediate targetCreated"
        );
        let infos = conn.browser_context.as_ref().unwrap().target_infos();
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0]["type"], "shared_worker");
        assert_eq!(infos[0]["attached"], false);
        let target_id = infos[0]["targetId"]
            .as_str()
            .expect("shared worker target id");
        assert_eq!(
            conn.target_registry_host_kind(target_id),
            Some(DevToolsTargetKind::SharedWorker)
        );
    }

    #[test]
    fn service_worker_target_registration_updates_browser_context_and_emits_when_discovered() {
        let mut conn = CdpConnection::default();
        conn.set_root_target_discovery_enabled(true);
        conn.browser_context = Some(BrowserContext::new("BID-1".to_owned()));

        let mut outputs =
            register_service_worker_target(&mut conn, "BID-1", service_worker_info(7))
                .worker_target_lifecycle_outputs
                .into_iter();
        let output = outputs
            .next()
            .expect("discovered service worker should emit targetCreated");
        let WorkerTargetLifecycleOutput::ServiceWorkerCreated { target_delta, .. } = output else {
            panic!("expected targetCreated output");
        };
        let target_id = target_delta.target_id().to_owned();
        assert!(outputs.next().is_none());
        let target_info = conn
            .browser_context
            .as_ref()
            .unwrap()
            .devtools_target_info(&target_id)
            .expect("created service worker target info");
        assert_eq!(target_info.kind, DevToolsTargetKind::ServiceWorker);
        assert_eq!(
            target_info.title,
            "Service Worker https://example.test/service-worker.js"
        );
        assert_eq!(target_info.url, "https://example.test/service-worker.js");
        assert_eq!(
            conn.target_registry_host_kind(&target_id),
            Some(DevToolsTargetKind::ServiceWorker)
        );

        let context = conn.browser_context.as_ref().unwrap();
        assert_eq!(
            context
                .target_info(&target_id)
                .expect("service worker target should be addressable")["type"],
            "service_worker"
        );
        assert!(
            register_service_worker_target(&mut conn, "BID-1", service_worker_info(7)).is_empty(),
            "duplicate created events for the same renderer version should not allocate a second target"
        );

        let outputs = remove_service_worker_target(&mut conn, "BID-1", 7, None)
            .worker_target_lifecycle_outputs;
        assert_eq!(
            outputs.first().and_then(destroyed_output_target_id),
            Some(target_id.as_str())
        );
        assert_eq!(outputs.len(), 1);
        assert_eq!(conn.target_registry_host_kind(&target_id), None);
        assert!(
            conn.browser_context
                .as_ref()
                .unwrap()
                .target_info(&target_id)
                .is_none()
        );
    }

    #[test]
    fn service_worker_target_registration_auto_attaches_when_enabled() {
        let mut conn = CdpConnection::default();
        conn.set_root_target_discovery_enabled(true);
        conn.set_auto_attach_owner(None, true, true, CdpTargetFilter::default_auto_attach());
        conn.browser_context = Some(BrowserContext::new("BID-1".to_owned()));

        let outputs = register_service_worker_target(&mut conn, "BID-1", service_worker_info(9))
            .worker_target_lifecycle_outputs;
        assert_eq!(outputs.len(), 2);
        let WorkerTargetLifecycleOutput::ServiceWorkerCreated { target_delta, .. } = &outputs[0]
        else {
            panic!("expected targetCreated before auto attach");
        };
        let target_id = target_delta.target_id().to_owned();
        let (attachment, prepared_attach) =
            service_worker_attached_output(&outputs[1]).expect("expected auto attached output");
        let session_id = attachment.session_id();
        let target_info = prepared_attach.target_info();
        assert_eq!(target_info_id(target_info), target_id.as_str());
        assert!(
            prepared_attach
                .sessions()
                .first()
                .is_some_and(crate::conn::TargetAttachSessionCommit::waiting_for_debugger),
            "service worker autoAttach output should preserve waitForDebuggerOnStart"
        );
        assert_eq!(
            conn.session_route(Some(session_id)),
            Some(crate::conn::CdpSessionRoute::ServiceWorkerTarget {
                browser_context_id: "BID-1".to_owned(),
                target_id: target_id.clone(),
            })
        );
    }

    #[tokio::test]
    async fn service_worker_target_lifecycle_drain_preserves_typed_target_sidecars() {
        let mut conn = CdpConnection::default();
        conn.set_target_discovery_for_owner(None, CdpTargetFilter::default_target_discovery());
        conn.set_auto_attach_owner(None, true, true, CdpTargetFilter::default_auto_attach());
        conn.browser_context = Some(BrowserContext::new("BID-1".to_owned()));

        let mut outputs =
            register_service_worker_target(&mut conn, "BID-1", service_worker_info(11));
        let (target_id, session_id) = outputs
            .worker_target_lifecycle_outputs
            .iter()
            .find_map(|output| {
                let (attachment, _) = service_worker_attached_output(output)?;
                Some((
                    attachment.target_id().to_owned(),
                    attachment.session_id().to_owned(),
                ))
            })
            .expect("service worker should be auto attached");
        outputs.extend(remove_service_worker_target(&mut conn, "BID-1", 11, None));

        let events = drain_target_lifecycle_events_for_test(&mut conn, outputs).await;
        let mut sidecars = Vec::new();
        for event in events {
            let (message, automation_event) = event.into_parts();
            match automation_event {
                Some(crate::devtools_runtime::AutomationEvent::TargetCreated(event)) => {
                    assert_eq!(message["method"], json!("Target.targetCreated"));
                    sidecars.push(("created", event.target_id.as_str().to_owned(), None));
                }
                Some(crate::devtools_runtime::AutomationEvent::TargetAttached(event)) => {
                    assert_eq!(message["method"], json!("Target.attachedToTarget"));
                    assert!(event.waiting_for_debugger);
                    sidecars.push((
                        "attached",
                        event.target_id.as_str().to_owned(),
                        Some(event.session_id.as_str().to_owned()),
                    ));
                }
                Some(crate::devtools_runtime::AutomationEvent::TargetDetached(event)) => {
                    assert_eq!(message["method"], json!("Target.detachedFromTarget"));
                    sidecars.push((
                        "detached",
                        event.target_id.as_str().to_owned(),
                        Some(event.session_id.as_str().to_owned()),
                    ));
                }
                Some(crate::devtools_runtime::AutomationEvent::TargetDestroyed(event)) => {
                    assert_eq!(message["method"], json!("Target.targetDestroyed"));
                    sidecars.push(("destroyed", event.target_id.as_str().to_owned(), None));
                }
                _ => {}
            }
        }

        assert_eq!(
            sidecars,
            vec![
                ("created", target_id.clone(), None),
                ("attached", target_id.clone(), Some(session_id.clone())),
                ("detached", target_id.clone(), Some(session_id)),
                ("destroyed", target_id, None),
            ]
        );
    }

    #[tokio::test]
    async fn shared_worker_target_lifecycle_drain_routes_discovery_events_to_owner_session() {
        let mut conn = CdpConnection::default();
        conn.browser_context = Some(BrowserContext::new("BID-1".to_owned()));
        conn.set_target_discovery_for_owner(
            Some("SID-browser"),
            CdpTargetFilter::default_target_discovery(),
        );

        let mut outputs =
            register_shared_worker_target(&mut conn, "BID-1", None, shared_worker_info(21));
        let target_id = outputs
            .worker_target_lifecycle_outputs
            .iter()
            .find_map(|output| match output {
                WorkerTargetLifecycleOutput::SharedWorkerCreated { target_delta } => {
                    Some(target_delta.target_id().to_owned())
                }
                _ => None,
            })
            .expect("shared worker target id");
        outputs.extend(remove_shared_worker_target(
            &mut conn,
            "BID-1",
            SharedWorkerInstanceId::from_u64(21),
        ));

        let out = drain_target_lifecycle_events_for_test(&mut conn, outputs)
            .await
            .into_iter()
            .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
            .collect::<Vec<_>>();

        assert!(
            out.iter().any(|message| {
                message["sessionId"] == json!("SID-browser")
                    && message["method"] == json!("Target.targetCreated")
                    && message["params"]["targetInfo"]["targetId"] == json!(target_id)
            }),
            "shared worker targetCreated should be routed to discovery owner: {out:?}"
        );
        assert!(
            out.iter().any(|message| {
                message["sessionId"] == json!("SID-browser")
                    && message["method"] == json!("Target.targetDestroyed")
                    && message["params"]["targetId"] == json!(target_id)
            }),
            "shared worker targetDestroyed should be routed to discovery owner: {out:?}"
        );
        assert!(
            !out.iter().any(|message| {
                message.get("sessionId").is_none()
                    && matches!(
                        message["method"].as_str(),
                        Some("Target.targetCreated" | "Target.targetDestroyed")
                    )
                    && (message["params"]["targetId"] == json!(target_id)
                        || message["params"]["targetInfo"]["targetId"] == json!(target_id))
            }),
            "worker discovery owner events must not leak as root events: {out:?}"
        );
    }

    #[test]
    fn service_worker_target_registration_respects_auto_attach_filter() {
        let mut conn = CdpConnection::default();
        conn.set_root_target_discovery_enabled(true);
        conn.browser_context = Some(BrowserContext::new("BID-1".to_owned()));
        conn.set_auto_attach_owner(None, true, false, target_filter_excluding("service_worker"));

        let outputs = register_service_worker_target(&mut conn, "BID-1", service_worker_info(10))
            .worker_target_lifecycle_outputs;
        assert_eq!(outputs.len(), 1);
        let WorkerTargetLifecycleOutput::ServiceWorkerCreated { target_delta, .. } = &outputs[0]
        else {
            panic!("discovery should still emit Target.targetCreated");
        };
        let target_id = target_delta.target_id().to_owned();
        let target = conn
            .browser_context
            .as_ref()
            .unwrap()
            .service_worker_target(&target_id)
            .expect("service worker target should be registered");
        assert!(!target.has_session());
    }

    #[test]
    fn service_worker_target_registration_auto_attaches_related_newer_version() {
        let mut conn = CdpConnection::default();
        conn.browser_context = Some(BrowserContext::new("BID-1".to_owned()));
        assert!(
            register_service_worker_target(&mut conn, "BID-1", service_worker_info(7)).is_empty()
        );

        conn.set_service_worker_auto_attach_related_owner(
            None,
            "BID-1",
            41,
            7,
            "https://example.test/service-worker.js".to_owned(),
            "https://example.test/app/".to_owned(),
            true,
            true,
        );

        assert!(
            register_service_worker_target(&mut conn, "BID-1", service_worker_info(6)).is_empty(),
            "older versions are not related autoAttach candidates"
        );
        let mut different_registration = service_worker_info(8);
        different_registration.registration_id = 42;
        assert!(
            register_service_worker_target(&mut conn, "BID-1", different_registration).is_empty(),
            "different registrations are not related autoAttach candidates"
        );

        let outputs = register_service_worker_target(&mut conn, "BID-1", service_worker_info(9))
            .worker_target_lifecycle_outputs;
        assert_eq!(outputs.len(), 1);
        let (attachment, prepared_attach) = service_worker_attached_output(&outputs[0])
            .expect("expected newer related service worker to auto attach");
        let session_id = attachment.session_id();
        let target_info = prepared_attach.target_info();
        let target_id = target_info_id_string(target_info);
        assert_eq!(target_info.kind, DevToolsTargetKind::ServiceWorker);
        assert!(
            prepared_attach
                .sessions()
                .first()
                .is_some_and(crate::conn::TargetAttachSessionCommit::waiting_for_debugger),
            "related autoAttach output should preserve waitForDebuggerOnStart"
        );
        assert!(matches!(
            conn.session_route(Some(session_id)),
            Some(crate::conn::CdpSessionRoute::ServiceWorkerTarget {
                browser_context_id,
                target_id: attached_target_id,
            }) if browser_context_id == "BID-1" && attached_target_id == target_id
        ));
    }

    #[test]
    fn service_worker_auto_attach_related_matching_is_context_scoped_and_preserves_wait() {
        let mut conn = CdpConnection::default();
        conn.browser_context = Some(BrowserContext::new("BID-1".to_owned()));
        conn.inactive_browser_contexts
            .push(BrowserContext::new("BID-2".to_owned()));
        conn.set_service_worker_auto_attach_related_owner(
            Some("SID-owner"),
            "BID-1",
            41,
            7,
            "https://example.test/service-worker.js".to_owned(),
            "https://example.test/app/".to_owned(),
            true,
            true,
        );

        let owners = conn.service_worker_auto_attach_related_owner_sessions_for_target(
            "BID-1",
            41,
            8,
            "https://example.test/service-worker.js",
            "https://example.test/app/",
        );
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].owner_session_id.as_deref(), Some("SID-owner"));
        assert!(
            owners[0].wait_for_debugger_on_start,
            "matched related owner should preserve waitForDebuggerOnStart"
        );
        assert!(
            conn.service_worker_auto_attach_related_owner_sessions_for_target(
                "BID-2",
                41,
                8,
                "https://example.test/service-worker.js",
                "https://example.test/app/",
            )
            .is_empty(),
            "same registration ids in another browser context must not match"
        );
    }

    #[test]
    fn target_creation_projects_starting_only_from_an_exact_live_host() {
        let mut conn = CdpConnection::default();
        let mut context = BrowserContext::new("BID-1".to_owned());
        context.set_active_target_id("TID-page".to_owned());
        context.attach_active_session("SID-page".to_owned());
        context.set_service_worker_domain_enabled(Some("SID-page"), true);
        conn.browser_context = Some(context);
        let run = renderer_run();

        let outputs = register_service_worker_target_with_active_run(
            &mut conn,
            "BID-1",
            service_worker_info(14),
            Some(run.clone()),
        )
        .worker_target_lifecycle_outputs;
        let created_version =
            lifecycle_protocol_message(&outputs, "ServiceWorker.workerVersionUpdated")
                .expect("an exact live host should be visible at target creation");
        assert_eq!(
            created_version["params"]["versions"][0]["runningStatus"],
            "starting"
        );

        let outputs = service_worker_target_lifecycle_outputs_for_events(
            &mut conn,
            "BID-1".to_owned(),
            vec![RendererServiceWorkerTargetEvent::Started {
                version_id: 14,
                run,
            }],
        )
        .worker_target_lifecycle_outputs;
        let started_version =
            lifecycle_protocol_message(&outputs, "ServiceWorker.workerVersionUpdated")
                .expect("the same exact run should transition to running");
        assert_eq!(
            started_version["params"]["versions"][0]["runningStatus"],
            "running"
        );
    }

    #[test]
    fn service_worker_domain_enabled_session_receives_target_lifecycle_updates() {
        let mut conn = CdpConnection::default();
        let mut context = BrowserContext::new("BID-1".to_owned());
        context.set_active_target_id("TID-page".to_owned());
        context.attach_active_session("SID-page".to_owned());
        context.set_service_worker_domain_enabled(Some("SID-page"), true);
        conn.browser_context = Some(context);

        let outputs = register_service_worker_target(&mut conn, "BID-1", service_worker_info(12))
            .worker_target_lifecycle_outputs;
        let registration =
            lifecycle_protocol_message(&outputs, "ServiceWorker.workerRegistrationUpdated")
                .expect("ServiceWorker domain should observe registration creation");
        assert_eq!(registration["sessionId"], "SID-page");
        assert_eq!(
            registration["params"]["registrations"][0],
            json!({
                "registrationId": "41",
                "scopeURL": "https://example.test/app/",
                "isDeleted": false
            })
        );
        let version = lifecycle_protocol_message(&outputs, "ServiceWorker.workerVersionUpdated")
            .expect("ServiceWorker domain should observe version creation");
        assert_eq!(version["sessionId"], "SID-page");
        assert_eq!(version["params"]["versions"][0]["versionId"], "12");
        assert_eq!(version["params"]["versions"][0]["runningStatus"], "stopped");
        assert_eq!(version["params"]["versions"][0]["status"], "installing");

        let renderer_run = renderer_run();
        let outputs = service_worker_target_lifecycle_outputs_for_events(
            &mut conn,
            "BID-1".to_owned(),
            vec![RendererServiceWorkerTargetEvent::Started {
                version_id: 12,
                run: renderer_run.clone(),
            }],
        )
        .worker_target_lifecycle_outputs;
        let started_version =
            lifecycle_protocol_message(&outputs, "ServiceWorker.workerVersionUpdated")
                .expect("ServiceWorker domain should observe worker start");
        assert_eq!(
            started_version["params"]["versions"][0]["runningStatus"],
            "running"
        );

        let outputs = service_worker_target_lifecycle_outputs_for_events(
            &mut conn,
            "BID-1".to_owned(),
            vec![RendererServiceWorkerTargetEvent::Stopped {
                version_id: 12,
                run: renderer_run,
                reason: "idle_timeout".to_owned(),
            }],
        )
        .worker_target_lifecycle_outputs;
        let stopped_version =
            lifecycle_protocol_message(&outputs, "ServiceWorker.workerVersionUpdated")
                .expect("ServiceWorker domain should observe worker stop");
        assert_eq!(
            stopped_version["params"]["versions"][0]["runningStatus"],
            "stopped"
        );

        let outputs = service_worker_target_lifecycle_outputs_for_events(
            &mut conn,
            "BID-1".to_owned(),
            vec![RendererServiceWorkerTargetEvent::VersionUpdated {
                version_id: 12,
                status: RendererServiceWorkerVersionStatus::Activated,
            }],
        )
        .worker_target_lifecycle_outputs;
        let updated_version =
            lifecycle_protocol_message(&outputs, "ServiceWorker.workerVersionUpdated")
                .expect("ServiceWorker domain should observe controlled client changes");
        assert_eq!(updated_version["sessionId"], "SID-page");
        assert_eq!(updated_version["params"]["versions"][0]["versionId"], "12");
        assert_eq!(
            updated_version["params"]["versions"][0]["controlledClients"],
            json!([])
        );
        assert_eq!(
            updated_version["params"]["versions"][0]["status"],
            "activated"
        );

        let outputs = remove_service_worker_target(&mut conn, "BID-1", 12, None)
            .worker_target_lifecycle_outputs;
        let deleted_registration =
            lifecycle_protocol_message(&outputs, "ServiceWorker.workerRegistrationUpdated")
                .expect("ServiceWorker domain should observe registration deletion");
        assert_eq!(
            deleted_registration["params"]["registrations"][0]["isDeleted"],
            true
        );
        let redundant_version =
            lifecycle_protocol_message(&outputs, "ServiceWorker.workerVersionUpdated")
                .expect("ServiceWorker domain should observe redundant version");
        assert_eq!(
            redundant_version["params"]["versions"][0]["status"],
            "redundant"
        );
    }

    #[test]
    fn service_worker_domain_destroyed_version_keeps_retained_registration() {
        let mut conn = CdpConnection::default();
        let mut context = BrowserContext::new("BID-1".to_owned());
        context.set_active_target_id("TID-page".to_owned());
        context.attach_active_session("SID-page".to_owned());
        context.set_service_worker_domain_enabled(Some("SID-page"), true);
        conn.browser_context = Some(context);
        let _ = register_service_worker_target(&mut conn, "BID-1", service_worker_info(12));
        let _ = register_service_worker_target(&mut conn, "BID-1", service_worker_info(13));

        let outputs = remove_service_worker_target(&mut conn, "BID-1", 12, None)
            .worker_target_lifecycle_outputs;
        assert!(
            lifecycle_protocol_message(&outputs, "ServiceWorker.workerRegistrationUpdated")
                .is_none(),
            "destroying a doomed old version must not delete a retained registration: {outputs:?}"
        );
        let redundant_version =
            lifecycle_protocol_message(&outputs, "ServiceWorker.workerVersionUpdated")
                .expect("ServiceWorker domain should observe old version redundancy");
        assert_eq!(redundant_version["sessionId"], "SID-page");
        assert_eq!(
            redundant_version["params"]["versions"][0]["versionId"],
            "12"
        );
        assert_eq!(
            redundant_version["params"]["versions"][0]["status"],
            "redundant"
        );

        let outputs = remove_service_worker_target(&mut conn, "BID-1", 13, None)
            .worker_target_lifecycle_outputs;
        let deleted_registration =
            lifecycle_protocol_message(&outputs, "ServiceWorker.workerRegistrationUpdated")
                .expect("last target removal should delete the registration projection");
        assert_eq!(
            deleted_registration["params"]["registrations"][0]["isDeleted"],
            true
        );
    }

    #[test]
    fn service_worker_domain_enabled_session_receives_worker_error_reported() {
        let mut conn = CdpConnection::default();
        let mut context = BrowserContext::new("BID-1".to_owned());
        context.set_active_target_id("TID-page".to_owned());
        context.attach_active_session("SID-page".to_owned());
        context.set_service_worker_domain_enabled(Some("SID-page"), true);
        conn.browser_context = Some(context);
        let _ = register_service_worker_target(&mut conn, "BID-1", service_worker_info(13));

        let outputs = service_worker_target_lifecycle_outputs_for_events(
            &mut conn,
            "BID-1".to_owned(),
            vec![RendererServiceWorkerTargetEvent::Exception {
                version_id: 13,
                run: renderer_run(),
                message: RendererServiceWorkerExceptionMessage {
                    message: "boom".to_owned(),
                    filename: "https://example.test/service-worker.js".to_owned(),
                    lineno: 9,
                    colno: 4,
                    event_kind: "fetch".to_owned(),
                    phase: "dispatch".to_owned(),
                    source: "exception".to_owned(),
                },
            }],
        )
        .worker_target_lifecycle_outputs;
        let error = lifecycle_protocol_message(&outputs, "ServiceWorker.workerErrorReported")
            .expect("ServiceWorker domain should observe worker error");
        assert_eq!(error["sessionId"], "SID-page");
        assert_eq!(error["params"]["errorMessage"]["errorMessage"], "boom");
        assert_eq!(error["params"]["errorMessage"]["registrationId"], "41");
        assert_eq!(error["params"]["errorMessage"]["versionId"], "13");
        assert_eq!(
            error["params"]["errorMessage"]["sourceURL"],
            "https://example.test/service-worker.js"
        );
        assert_eq!(error["params"]["errorMessage"]["lineNumber"], 9);
        assert_eq!(error["params"]["errorMessage"]["columnNumber"], 4);
    }

    #[test]
    fn service_worker_target_destruction_detaches_session_before_destroyed() {
        let mut conn = CdpConnection::default();
        conn.auto_attach = true;
        conn.set_root_target_discovery_enabled(true);
        conn.browser_context = Some(BrowserContext::new("BID-1".to_owned()));

        let outputs = register_service_worker_target(&mut conn, "BID-1", service_worker_info(11))
            .worker_target_lifecycle_outputs;
        let (target_id, attached_session_id) = outputs
            .iter()
            .find_map(|output| {
                let (attachment, _) = service_worker_attached_output(output)?;
                Some((
                    attachment.target_id().to_owned(),
                    attachment.session_id().to_owned(),
                ))
            })
            .expect("auto-attached target id");
        let outputs = remove_service_worker_target(&mut conn, "BID-1", 11, None)
            .worker_target_lifecycle_outputs;

        let WorkerTargetLifecycleOutput::ServiceWorkerDetached {
            retirement,
            cleanup_plan,
        } = &outputs[0]
        else {
            panic!("service-worker attachment must retire before target destruction");
        };
        assert_eq!(retirement.identity().target_id(), target_id);
        assert_eq!(retirement.identity().session_id(), attached_session_id);
        assert_eq!(cleanup_plan.target_id(), target_id);
        assert_eq!(cleanup_plan.session_id(), attached_session_id);
        assert_eq!(
            destroyed_output_target_id(&outputs[1]),
            Some(target_id.as_str())
        );
        assert_eq!(outputs.len(), 2);
        assert!(
            conn.browser_context
                .as_ref()
                .unwrap()
                .target_info(&target_id)
                .is_none()
        );
    }

    #[test]
    fn service_worker_target_stopped_retains_target_and_clears_runtime_sessions() {
        let mut conn = CdpConnection::default();
        conn.auto_attach = true;
        conn.set_root_target_discovery_enabled(true);
        conn.browser_context = Some(BrowserContext::new("BID-1".to_owned()));

        let outputs = register_service_worker_target(&mut conn, "BID-1", service_worker_info(17))
            .worker_target_lifecycle_outputs;
        let (target_id, attached_session_id) = outputs
            .iter()
            .find_map(|output| {
                let (attachment, _) = service_worker_attached_output(output)?;
                Some((
                    attachment.target_id().to_owned(),
                    attachment.session_id().to_owned(),
                ))
            })
            .expect("auto-attached service worker target id");
        let target = conn
            .service_worker_target_for_session_mut(Some(&attached_session_id))
            .expect("service worker target should be attached");
        target.set_runtime_frontend_enabled(&attached_session_id, true);
        target.set_inspector_enabled(&attached_session_id, true);
        target.record_runtime_contexts_reported_to_frontend(&attached_session_id);
        target.register_pending_inspector_await(
            &attached_session_id,
            917,
            Some(&attached_session_id),
            None,
        );

        let renderer_run = renderer_run();
        let outputs = record_service_worker_target_stopped(
            &mut conn,
            "BID-1",
            17,
            renderer_run,
            "idle_timeout".to_owned(),
        )
        .worker_target_lifecycle_outputs;

        let await_failure = outputs
            .iter()
            .find_map(|output| match output {
                WorkerTargetLifecycleOutput::ServiceWorkerRuntimeEvents { events, .. } => events
                    .iter()
                    .cloned()
                    .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
                    .find(|message| message["id"] == json!(917)),
                _ => None,
            })
            .expect("service worker stop should fail pending inspector await");
        assert_eq!(await_failure["sessionId"], attached_session_id);
        assert_eq!(
            await_failure["error"]["message"],
            json!("Service worker stopped")
        );
        let runtime_cleared_event = outputs
            .iter()
            .find_map(|output| match output {
                WorkerTargetLifecycleOutput::ServiceWorkerRuntimeEvents { events, .. } => events
                    .iter()
                    .find(|&event| {
                        event.protocol_method() == Some("Runtime.executionContextsCleared")
                    })
                    .cloned(),
                _ => None,
            })
            .expect("service worker stop should clear reported Runtime contexts");
        let (runtime_cleared, runtime_cleared_sidecar) = runtime_cleared_event.into_parts();
        assert_eq!(runtime_cleared["sessionId"], attached_session_id);
        assert!(matches!(
            runtime_cleared_sidecar,
            Some(crate::devtools_runtime::AutomationEvent::RuntimeExecutionContextsCleared(_))
        ));
        let inspector_crashed = lifecycle_protocol_message(&outputs, "Inspector.targetCrashed")
            .expect("service worker stop should emit Inspector.targetCrashed");
        assert_eq!(inspector_crashed["sessionId"], attached_session_id);
        assert!(
            outputs.iter().all(|output| !matches!(
                output,
                WorkerTargetLifecycleOutput::ServiceWorkerDetached { .. }
                    | WorkerTargetLifecycleOutput::ServiceWorkerDestroyed { .. }
            )),
            "service worker stop should retain target/session: {outputs:?}"
        );
        let target = conn
            .service_worker_target_for_session(Some(&attached_session_id))
            .expect("stopped service worker target should stay attached");
        assert_eq!(target.target_id, target_id);
        assert!(
            target.runtime_context_reported_session_ids().is_empty(),
            "Runtime context should be cleared for the stopped worker"
        );
    }

    #[test]
    fn service_worker_target_started_reloads_after_crash_once() {
        let mut conn = CdpConnection::default();
        conn.auto_attach = true;
        conn.set_root_target_discovery_enabled(true);
        conn.browser_context = Some(BrowserContext::new("BID-1".to_owned()));

        let outputs = register_service_worker_target(&mut conn, "BID-1", service_worker_info(19))
            .worker_target_lifecycle_outputs;
        let attached_session_id = outputs
            .iter()
            .find_map(|output| {
                service_worker_attached_output(output)
                    .map(|(attachment, _)| attachment.session_id().to_owned())
            })
            .expect("auto-attached service worker target id");
        let target = conn
            .service_worker_target_for_session_mut(Some(&attached_session_id))
            .expect("service worker target should be attached");
        target.set_inspector_enabled(&attached_session_id, true);

        let first_run = renderer_run();
        let stopped = record_service_worker_target_stopped(
            &mut conn,
            "BID-1",
            19,
            first_run,
            "idle_timeout".to_owned(),
        )
        .worker_target_lifecycle_outputs;
        let crashed = lifecycle_protocol_message(&stopped, "Inspector.targetCrashed")
            .expect("stopped service worker should notify inspector crash");
        assert_eq!(crashed["sessionId"], attached_session_id);

        let second_run = renderer_run();
        let started =
            record_service_worker_target_started(&mut conn, "BID-1", 19, second_run.clone())
                .worker_target_lifecycle_outputs;
        let reloaded = lifecycle_protocol_message(&started, "Inspector.targetReloadedAfterCrash")
            .expect("started service worker should notify inspector reload after crash");
        assert_eq!(reloaded["sessionId"], attached_session_id);

        assert!(
            record_service_worker_target_started(&mut conn, "BID-1", 19, second_run).is_empty(),
            "duplicate started event for the same exact run must not replay reload"
        );
    }

    #[tokio::test]
    async fn service_worker_restart_emits_reload_before_new_runtime_context() {
        let mut conn = CdpConnection::default();
        conn.auto_attach = true;
        conn.set_root_target_discovery_enabled(true);
        conn.browser_context = Some(BrowserContext::new("BID-1".to_owned()));

        let outputs = register_service_worker_target(&mut conn, "BID-1", service_worker_info(29))
            .worker_target_lifecycle_outputs;
        let (target_id, attached_session_id) = outputs
            .iter()
            .find_map(|output| {
                let (attachment, _) = service_worker_attached_output(output)?;
                Some((
                    attachment.target_id().to_owned(),
                    attachment.session_id().to_owned(),
                ))
            })
            .expect("auto-attached service worker target id");
        let target = conn
            .service_worker_target_for_session_mut(Some(&attached_session_id))
            .expect("service worker target should be attached");
        target.set_runtime_frontend_enabled(&attached_session_id, true);
        target.set_inspector_enabled(&attached_session_id, true);

        let first_run = renderer_run();
        let outputs = service_worker_target_lifecycle_outputs_for_events(
            &mut conn,
            "BID-1".to_owned(),
            vec![RendererServiceWorkerTargetEvent::RuntimeInspectorMessages {
                version_id: 29,
                run: first_run.clone(),
                inspector_session_id: None,
                messages: runtime_inspector_messages(vec![json!({
                    "method": "Runtime.executionContextCreated",
                    "params": {
                        "context": {
                            "id": 2901,
                            "uniqueId": "service-worker-realm-1",
                            "origin": "https://example.test",
                            "name": "",
                            "auxData": {
                                "isDefault": true,
                                "type": "service-worker"
                            }
                        }
                    }
                })]),
            }],
        );
        let out = drain_target_lifecycle_events_for_test(&mut conn, outputs)
            .await
            .into_iter()
            .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
            .collect::<Vec<_>>();
        assert!(
            out.iter().any(|message| {
                message["method"] == json!("Runtime.executionContextCreated")
                    && message["sessionId"] == json!(attached_session_id)
                    && message["params"]["context"]["id"] == json!(2901)
                    && message["params"]["context"]["uniqueId"]
                        == json!(format!("{target_id}:service-worker-realm-1"))
            }),
            "initial service worker Runtime context should be emitted: {out:?}"
        );

        let second_run = renderer_run();
        let outputs = service_worker_target_lifecycle_outputs_for_events(
            &mut conn,
            "BID-1".to_owned(),
            vec![
                RendererServiceWorkerTargetEvent::Stopped {
                    version_id: 29,
                    run: first_run,
                    reason: "idle_timeout".to_owned(),
                },
                RendererServiceWorkerTargetEvent::Started {
                    version_id: 29,
                    run: second_run.clone(),
                },
                RendererServiceWorkerTargetEvent::RuntimeInspectorMessages {
                    version_id: 29,
                    run: second_run,
                    inspector_session_id: None,
                    messages: runtime_inspector_messages(vec![json!({
                        "method": "Runtime.executionContextCreated",
                        "params": {
                            "context": {
                                "id": 2902,
                                "uniqueId": "service-worker-realm-2",
                                "origin": "https://example.test",
                                "name": "",
                                "auxData": {
                                    "isDefault": true,
                                    "type": "service-worker"
                                }
                            }
                        }
                    })]),
                },
            ],
        );
        let out = drain_target_lifecycle_events_for_test(&mut conn, outputs)
            .await
            .into_iter()
            .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
            .collect::<Vec<_>>();

        let lifecycle_methods: Vec<_> = out
            .iter()
            .filter(|message| message["sessionId"] == json!(attached_session_id))
            .filter_map(|message| message["method"].as_str())
            .filter(|method| {
                matches!(
                    *method,
                    "Inspector.targetCrashed"
                        | "Runtime.executionContextsCleared"
                        | "Inspector.targetReloadedAfterCrash"
                        | "Runtime.executionContextCreated"
                )
            })
            .collect();
        assert_eq!(
            lifecycle_methods,
            vec![
                "Inspector.targetCrashed",
                "Runtime.executionContextsCleared",
                "Inspector.targetReloadedAfterCrash",
                "Runtime.executionContextCreated",
            ],
            "service worker restart should mirror Chromium crash/reload/context order: {out:?}"
        );
        let runtime_context = out
            .iter()
            .find(|message| {
                message["method"] == json!("Runtime.executionContextCreated")
                    && message["sessionId"] == json!(attached_session_id)
            })
            .expect("restarted service worker should expose a fresh Runtime context");
        assert_eq!(runtime_context["params"]["context"]["id"], json!(2902));
        assert_eq!(
            runtime_context["params"]["context"]["uniqueId"],
            json!(format!("{target_id}:service-worker-realm-2"))
        );
        let target = conn
            .service_worker_target_for_session(Some(&attached_session_id))
            .expect("service worker target should remain attached after restart");
        assert_eq!(target.target_id, target_id);
        assert_eq!(target.execution_context_id(), 2902);
        assert_eq!(
            target.runtime_context_reported_session_ids(),
            vec![attached_session_id]
        );
    }

    #[test]
    fn service_worker_version_state_updates_without_a_domain_listener() {
        let mut conn = CdpConnection::default();
        conn.browser_context = Some(BrowserContext::new("BID-1".to_owned()));
        assert!(
            register_service_worker_target(&mut conn, "BID-1", service_worker_info(31)).is_empty(),
            "an undiscovered target without domain listeners should not manufacture output"
        );

        let outputs = record_service_worker_target_version_updated(
            &mut conn,
            "BID-1",
            31,
            RendererServiceWorkerVersionStatus::Activated,
        );

        assert!(
            outputs.is_empty(),
            "protocol listener absence should suppress output, not the owner state transition"
        );
        let target = conn
            .browser_context
            .as_ref()
            .and_then(|context| {
                context
                    .service_worker_target_id_for_renderer_version(31)
                    .and_then(|target_id| context.service_worker_target(target_id))
            })
            .expect("service-worker target should remain registered");
        assert_eq!(target.version_status_cdp_str(), "activated");
    }

    #[test]
    fn stale_destroy_cannot_remove_a_restarted_service_worker_version() {
        let mut conn = CdpConnection::default();
        conn.browser_context = Some(BrowserContext::new("BID-1".to_owned()));
        let _ = register_service_worker_target(&mut conn, "BID-1", service_worker_info(32));
        let old_run = renderer_run();
        let old_run_retirement = record_service_worker_target_stopped(
            &mut conn,
            "BID-1",
            32,
            old_run.clone(),
            "idle".to_owned(),
        );
        let new_run = renderer_run();
        let _ = record_service_worker_target_started(&mut conn, "BID-1", 32, new_run);

        assert!(
            remove_service_worker_target(&mut conn, "BID-1", 32, Some(old_run)).is_empty(),
            "a late destruction for the previous run must not retire the stable version target"
        );
        let target = conn
            .browser_context
            .as_ref()
            .and_then(|context| {
                context
                    .service_worker_target_id_for_renderer_version(32)
                    .and_then(|target_id| context.service_worker_target(target_id))
            })
            .expect("restarted service-worker target should remain registered");
        assert_eq!(target.running_status_cdp_str(), "running");

        drop(old_run_retirement);
    }

    #[tokio::test]
    async fn retired_run_output_never_replays_into_the_restarted_worker() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-1".to_owned());
        let mut target = ServiceWorkerTargetState::new(
            41,
            33,
            "TID-service-worker".to_owned(),
            "https://example.test/service-worker.js".to_owned(),
            "https://example.test/".to_owned(),
            RendererServiceWorkerVersionStatus::Activated,
            None,
        );
        target.attach_session("SID-service-worker".to_owned());
        target.set_console_enabled("SID-service-worker", true);
        browser_context.insert_service_worker_target(target);
        conn.browser_context = Some(browser_context);

        let old_run = renderer_run();
        let old_output = record_service_worker_target_console_message(
            &mut conn,
            "BID-1",
            33,
            old_run.clone(),
            RendererServiceWorkerConsoleMessage {
                message: "log: old run".to_owned(),
                args: Vec::new(),
                stack: None,
            },
        );
        let retirement = record_service_worker_target_stopped(
            &mut conn,
            "BID-1",
            33,
            old_run,
            "idle".to_owned(),
        );
        assert!(
            drain_target_lifecycle_events_for_test(&mut conn, retirement)
                .await
                .is_empty(),
            "run retirement without enabled lifecycle domains should be protocol-silent"
        );
        let new_run = renderer_run();
        assert!(
            record_service_worker_target_started(&mut conn, "BID-1", 33, new_run.clone())
                .is_empty(),
            "restart without Inspector or ServiceWorker listeners should be protocol-silent"
        );

        assert!(
            drain_target_lifecycle_events_for_test(&mut conn, old_output)
                .await
                .is_empty(),
            "output accepted for the retired run must not bind to the new run"
        );

        let new_output = record_service_worker_target_console_message(
            &mut conn,
            "BID-1",
            33,
            new_run,
            RendererServiceWorkerConsoleMessage {
                message: "log: new run".to_owned(),
                args: Vec::new(),
                stack: None,
            },
        );
        let messages = drain_target_lifecycle_events_for_test(&mut conn, new_output)
            .await
            .into_iter()
            .map(BackgroundProtocolEvent::into_protocol_message)
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["method"], "Console.messageAdded");
        assert_eq!(messages[0]["params"]["message"]["text"], "new run");
    }

    #[tokio::test]
    async fn detached_session_id_reuse_does_not_revive_captured_worker_output() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-1".to_owned());
        let mut target = ServiceWorkerTargetState::new(
            41,
            34,
            "TID-service-worker".to_owned(),
            "https://example.test/service-worker.js".to_owned(),
            "https://example.test/".to_owned(),
            RendererServiceWorkerVersionStatus::Activated,
            None,
        );
        target.attach_session("SID-reused".to_owned());
        target.set_console_enabled("SID-reused", true);
        browser_context.insert_service_worker_target(target);
        conn.browser_context = Some(browser_context);

        let old_output = record_service_worker_target_console_message(
            &mut conn,
            "BID-1",
            34,
            renderer_run(),
            RendererServiceWorkerConsoleMessage {
                message: "log: old attachment".to_owned(),
                args: Vec::new(),
                stack: None,
            },
        );
        let target = conn
            .browser_context
            .as_mut()
            .and_then(|context| context.service_worker_target_mut("TID-service-worker"))
            .expect("service-worker target should remain resident");
        assert!(target.detach_session("SID-reused"));
        target.attach_session("SID-reused".to_owned());
        target.set_console_enabled("SID-reused", true);

        assert!(
            drain_target_lifecycle_events_for_test(&mut conn, old_output)
                .await
                .is_empty(),
            "a new attachment with the same scalar session id must not authorize old output"
        );
    }

    #[test]
    fn shared_worker_target_registration_auto_attaches_when_enabled() {
        let mut conn = CdpConnection::default();
        conn.auto_attach = true;
        conn.browser_context = Some(BrowserContext::new("BID-1".to_owned()));

        let outputs =
            register_shared_worker_target(&mut conn, "BID-1", None, shared_worker_info(11))
                .worker_target_lifecycle_outputs;
        assert_eq!(outputs.len(), 1);
        let (attachment, prepared_attach) = shared_worker_attached_output(&outputs[0])
            .expect("auto-attach should prepare an exact Target.attachedToTarget");
        assert!(prepared_attach.target_info().attached);
        assert_eq!(
            attachment.target_id(),
            target_info_id(prepared_attach.target_info())
        );
        assert!(matches!(
            conn.session_route(Some(attachment.session_id())),
            Some(crate::conn::CdpSessionRoute::SharedWorkerTarget { .. })
        ));
        assert!(attachment.is_current());
    }

    #[test]
    fn shared_worker_registration_only_auto_attaches_browser_level_owners() {
        let (mut conn, owner_page, owner_renderer_page) = dedicated_worker_fixture();
        assert!(
            conn.prepare_auto_attached_page_session_binding("TID-page", "SID-page".to_owned(),)
        );
        conn.register_browser_session("SID-browser-1".to_owned());
        conn.register_browser_session("SID-browser-2".to_owned());

        conn.browser_context
            .as_mut()
            .expect("browser context")
            .insert_dedicated_worker_target(DedicatedWorkerTargetState::new(
                owner_page,
                owner_renderer_page.owner_local_host_id(),
                91,
                "TID-dedicated-worker".to_owned(),
                "https://example.test/worker.js".to_owned(),
                Vec::new(),
            ));
        assert!(conn.prepare_auto_attached_dedicated_worker_session_binding(
            "TID-dedicated-worker",
            "SID-dedicated-worker".to_owned(),
        ));

        let mut owner_shared_worker = SharedWorkerTargetState::new(
            moli_core::RendererOwnerLocalHostId::new_for_testing(2),
            SharedWorkerInstanceId::from_u64(92),
            "TID-owner-shared-worker".to_owned(),
            None,
            "https://example.test/owner-shared-worker.js".to_owned(),
            "owner".to_owned(),
        );
        owner_shared_worker.attach_session("SID-shared-worker".to_owned());
        conn.browser_context
            .as_mut()
            .expect("browser context")
            .insert_shared_worker_target(owner_shared_worker);

        for (owner, wait_for_debugger_on_start) in [
            (None, false),
            (Some("SID-browser-1"), false),
            (Some("SID-browser-2"), true),
            (Some("SID-page"), true),
            (Some("SID-dedicated-worker"), true),
            (Some("SID-shared-worker"), true),
        ] {
            conn.set_auto_attach_owner(
                owner,
                true,
                wait_for_debugger_on_start,
                CdpTargetFilter::default_auto_attach(),
            );
        }

        let outputs =
            register_shared_worker_target(&mut conn, "BID-1", None, shared_worker_info(93))
                .worker_target_lifecycle_outputs;
        let mut attached_owners = outputs
            .iter()
            .filter_map(shared_worker_attached_output)
            .map(|(_, prepared_attach)| {
                assert_eq!(prepared_attach.sessions().len(), 1);
                let (_, owner, _, _, waiting_for_debugger) =
                    prepared_attach.sessions()[0].clone().into_parts();
                (owner, waiting_for_debugger)
            })
            .collect::<Vec<_>>();
        attached_owners.sort();

        assert_eq!(
            attached_owners,
            vec![
                (None, false),
                (Some("SID-browser-1".to_owned()), false),
                (Some("SID-browser-2".to_owned()), true),
            ],
            "page and worker TargetHandlers must not receive browser-level shared workers"
        );
    }

    #[test]
    fn shared_worker_target_registration_respects_auto_attach_filter() {
        let mut conn = CdpConnection::default();
        conn.set_root_target_discovery_enabled(true);
        conn.browser_context = Some(BrowserContext::new("BID-1".to_owned()));
        conn.set_auto_attach_owner(None, true, false, target_filter_excluding("shared_worker"));

        let outputs =
            register_shared_worker_target(&mut conn, "BID-1", None, shared_worker_info(12))
                .worker_target_lifecycle_outputs;
        assert_eq!(outputs.len(), 1);
        let WorkerTargetLifecycleOutput::SharedWorkerCreated { target_delta } = &outputs[0] else {
            panic!("discovery should still emit Target.targetCreated");
        };
        let target_id = target_delta.target_id().to_owned();
        let target = conn
            .browser_context
            .as_ref()
            .unwrap()
            .shared_worker_target(&target_id)
            .expect("shared worker target should be registered");
        assert!(!target.has_session());
    }

    #[test]
    fn shared_worker_runtime_inspector_messages_route_to_tagged_session_only() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-1".to_owned());
        let instance_id = SharedWorkerInstanceId::from_u64(12);
        let mut target = SharedWorkerTargetState::new(
            moli_core::RendererOwnerLocalHostId::new_for_testing(1),
            instance_id,
            "TID-shared-worker".to_owned(),
            None,
            "https://example.test/shared-worker.js".to_owned(),
            "worker".to_owned(),
        );
        target.attach_session("SID-first".to_owned());
        target.attach_session("SID-second".to_owned());
        browser_context.insert_shared_worker_target(target);
        conn.browser_context = Some(browser_context);

        let outputs = record_shared_worker_target_runtime_inspector_messages(
            &mut conn,
            "BID-1",
            instance_id,
            Some("SID-second".to_owned()),
            runtime_inspector_messages(vec![json!({"method": "Runtime.executionContextCreated"})]),
        )
        .worker_target_lifecycle_outputs;

        assert_eq!(outputs.len(), 1);
        let WorkerTargetLifecycleOutput::SharedWorkerRuntimeInspectorMessages {
            attachment,
            messages,
        } = &outputs[0]
        else {
            panic!("shared-worker inspector output must retain its exact attachment");
        };
        assert_eq!(attachment.target_id(), "TID-shared-worker");
        assert_eq!(attachment.session_id(), "SID-second");
        assert!(attachment.is_current());
        assert_eq!(
            messages,
            &runtime_inspector_messages(vec![json!({
                "method": "Runtime.executionContextCreated"
            })])
        );
    }

    #[test]
    fn service_worker_console_messages_bind_the_exact_run_and_enabled_sessions() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-1".to_owned());
        let renderer_run = renderer_run();
        let mut target = ServiceWorkerTargetState::new(
            41,
            22,
            "TID-service-worker".to_owned(),
            "https://example.test/service-worker.js".to_owned(),
            "https://example.test/".to_owned(),
            RendererServiceWorkerVersionStatus::Activated,
            Some(renderer_run.clone()),
        );
        target.attach_session("SID-first".to_owned());
        target.attach_session("SID-second".to_owned());
        target.set_console_enabled("SID-second", true);
        target.set_runtime_frontend_enabled("SID-second", true);
        target.record_runtime_execution_context_created_event(&worker_context_created_event(
            90_022,
            "service-worker",
        ));
        browser_context.insert_service_worker_target(target);
        conn.browser_context = Some(browser_context);

        let outputs = record_service_worker_target_console_message(
            &mut conn,
            "BID-1",
            22,
            renderer_run,
            RendererServiceWorkerConsoleMessage {
                message: "log: ready".to_owned(),
                args: vec![json!({"type": "string", "value": "ready"})],
                stack: None,
            },
        )
        .worker_target_lifecycle_outputs;

        assert_eq!(outputs.len(), 2);
        let WorkerTargetLifecycleOutput::ServiceWorkerConsoleMessages {
            runtime,
            messages,
            console_end,
        } = &outputs[0]
        else {
            panic!("Console output must retain its exact ServiceWorker run");
        };
        assert_eq!(runtime.target_id(), "TID-service-worker");
        assert_eq!(runtime.session_id(), "SID-second");
        assert!(runtime.is_current());
        assert_eq!(*console_end, 1);
        assert_eq!(
            messages,
            &[RuntimeConsoleMessageSnapshot {
                execution_context_id: 90_022,
                message: "log: ready".to_owned(),
                args: vec![json!({"type": "string", "value": "ready"})],
                stack: None,
            }]
        );
        let WorkerTargetLifecycleOutput::ServiceWorkerRuntimeConsoleMessages {
            runtime,
            messages,
            console_end,
        } = &outputs[1]
        else {
            panic!("Runtime output must retain its exact ServiceWorker run");
        };
        assert_eq!(runtime.session_id(), "SID-second");
        assert_eq!(*console_end, 1);
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn service_worker_exception_messages_bind_the_exact_run_and_enabled_sessions() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-1".to_owned());
        let renderer_run = renderer_run();
        let mut target = ServiceWorkerTargetState::new(
            41,
            24,
            "TID-service-worker".to_owned(),
            "https://example.test/service-worker.js".to_owned(),
            "https://example.test/".to_owned(),
            RendererServiceWorkerVersionStatus::Activated,
            Some(renderer_run.clone()),
        );
        target.attach_session("SID-first".to_owned());
        target.attach_session("SID-second".to_owned());
        target.set_runtime_frontend_enabled("SID-second", true);
        target.record_runtime_execution_context_created_event(&worker_context_created_event(
            90_024,
            "service-worker",
        ));
        browser_context.insert_service_worker_target(target);
        conn.browser_context = Some(browser_context);

        let exception_message = RendererServiceWorkerExceptionMessage {
            message: "Uncaught Error: boom".to_owned(),
            filename: "https://example.test/service-worker.js".to_owned(),
            lineno: 3,
            colno: 9,
            event_kind: "error_event".to_owned(),
            phase: "runtime".to_owned(),
            source: "runtime".to_owned(),
        };
        let outputs = record_service_worker_target_exception_message(
            &mut conn,
            "BID-1",
            24,
            renderer_run,
            exception_message.clone(),
        )
        .worker_target_lifecycle_outputs;

        assert_eq!(outputs.len(), 1);
        let WorkerTargetLifecycleOutput::ServiceWorkerRuntimeExceptionMessages {
            runtime,
            messages,
            exception_start,
            exception_end,
        } = &outputs[0]
        else {
            panic!("exception output must retain its exact ServiceWorker run");
        };
        assert_eq!(runtime.target_id(), "TID-service-worker");
        assert_eq!(runtime.session_id(), "SID-second");
        assert_eq!(*exception_start, 0);
        assert_eq!(*exception_end, 1);
        assert_eq!(
            messages,
            &[ServiceWorkerRuntimeExceptionSnapshot {
                execution_context_id: 90_024,
                message: exception_message,
            }]
        );
    }

    #[test]
    fn service_worker_exception_messages_emit_runtime_exception_thrown_shape() {
        let messages = vec![ServiceWorkerRuntimeExceptionSnapshot {
            execution_context_id: 90_024,
            message: RendererServiceWorkerExceptionMessage {
                message: "Uncaught Error: boom".to_owned(),
                filename: "https://example.test/service-worker.js".to_owned(),
                lineno: 3,
                colno: 9,
                event_kind: "error_event".to_owned(),
                phase: "runtime".to_owned(),
                source: "runtime".to_owned(),
            },
        }];

        let out = runtime_exception_thrown_events("SID-worker", &messages, 7);

        assert_eq!(out.len(), 1);
        let (message, automation_event) = out.into_iter().next().unwrap().into_parts();
        assert_eq!(message["sessionId"], json!("SID-worker"));
        assert_eq!(message["method"], json!("Runtime.exceptionThrown"));
        let details = &message["params"]["exceptionDetails"];
        assert_eq!(details["exceptionId"], json!(8));
        assert_eq!(details["text"], json!("Uncaught Error: boom"));
        assert_eq!(
            details["url"],
            json!("https://example.test/service-worker.js")
        );
        assert_eq!(details["executionContextId"], json!(90_024));
        assert_eq!(details["lineNumber"], json!(2));
        assert_eq!(details["columnNumber"], json!(8));
        let Some(crate::devtools_runtime::AutomationEvent::ScriptException(event)) =
            automation_event
        else {
            panic!("expected ScriptException sidecar");
        };
        assert_eq!(event.exception.text, "Uncaught Error: boom");
        assert_eq!(event.exception.line_number, Some(2));
        assert_eq!(event.exception.column_number, Some(8));
    }

    #[test]
    fn service_worker_fetch_diagnostics_bind_the_exact_run_and_enabled_sessions() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-1".to_owned());
        let mut target = ServiceWorkerTargetState::new(
            41,
            25,
            "TID-service-worker".to_owned(),
            "https://example.test/service-worker.js".to_owned(),
            "https://example.test/".to_owned(),
            RendererServiceWorkerVersionStatus::Activated,
            None,
        );
        target.attach_session("SID-first".to_owned());
        target.attach_session("SID-second".to_owned());
        assert!(target.set_network_enabled("SID-second", true));
        browser_context.insert_service_worker_target(target);
        conn.browser_context = Some(browser_context);

        let diagnostic = RendererServiceWorkerFetchDiagnostic {
            internal_id: 101,
            document_url: "https://example.test/app/".to_owned(),
            request_url: "https://example.test/api".to_owned(),
            method: "POST".to_owned(),
            request_headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
            request_body: Some("hello".to_owned()),
            destination: "".to_owned(),
            result: RendererServiceWorkerFetchDiagnosticResult::Response {
                final_url: "https://example.test/api".to_owned(),
                status: 201,
                status_text: "Created".to_owned(),
                response_headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
                body_len: 2,
            },
        };
        let renderer_run = renderer_run();
        let outputs = record_service_worker_target_fetch_diagnostic(
            &mut conn,
            "BID-1",
            25,
            renderer_run,
            diagnostic.clone(),
        )
        .worker_target_lifecycle_outputs;

        assert_eq!(outputs.len(), 1);
        let WorkerTargetLifecycleOutput::ServiceWorkerFetchDiagnostics {
            runtime,
            diagnostics,
            diagnostic_start,
            diagnostic_end,
        } = &outputs[0]
        else {
            panic!("fetch diagnostic must retain its exact ServiceWorker run");
        };
        assert_eq!(runtime.target_id(), "TID-service-worker");
        assert_eq!(runtime.session_id(), "SID-second");
        assert_eq!(diagnostics, &[diagnostic]);
        assert_eq!(*diagnostic_start, 0);
        assert_eq!(*diagnostic_end, 1);
    }

    #[test]
    fn service_worker_fetch_diagnostics_emit_network_event_shape() {
        let diagnostics = vec![
            RendererServiceWorkerFetchDiagnostic {
                internal_id: 101,
                document_url: "https://example.test/app/".to_owned(),
                request_url: "https://example.test/api".to_owned(),
                method: "POST".to_owned(),
                request_headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
                request_body: Some("hello".to_owned()),
                destination: "".to_owned(),
                result: RendererServiceWorkerFetchDiagnosticResult::Response {
                    final_url: "https://example.test/api".to_owned(),
                    status: 201,
                    status_text: "Created".to_owned(),
                    response_headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
                    body_len: 2,
                },
            },
            RendererServiceWorkerFetchDiagnostic {
                internal_id: 102,
                document_url: "https://example.test/app/".to_owned(),
                request_url: "https://example.test/fallback".to_owned(),
                method: "GET".to_owned(),
                request_headers: Vec::new(),
                request_body: None,
                destination: "script".to_owned(),
                result: RendererServiceWorkerFetchDiagnosticResult::Fallback,
            },
            RendererServiceWorkerFetchDiagnostic {
                internal_id: 103,
                document_url: "https://example.test/app/".to_owned(),
                request_url: "https://example.test/failure".to_owned(),
                method: "GET".to_owned(),
                request_headers: Vec::new(),
                request_body: None,
                destination: "image".to_owned(),
                result: RendererServiceWorkerFetchDiagnosticResult::Failure {
                    message: "network down".to_owned(),
                },
            },
        ];
        let events = service_worker_fetch_diagnostic_events(
            "SID-worker",
            "TID-service-worker",
            &diagnostics,
            4,
        );
        let mut out = Vec::new();
        let mut sidecars = Vec::new();
        for event in events {
            let (message, sidecar) = event.into_parts();
            out.push(message);
            sidecars.push(sidecar);
        }

        let methods = out
            .iter()
            .map(|message| message["method"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            methods,
            vec![
                "Network.requestWillBeSent",
                "Network.responseReceived",
                "Network.dataReceived",
                "Network.loadingFinished",
                "Network.requestWillBeSent",
                "Network.loadingFailed",
                "Network.requestWillBeSent",
                "Network.loadingFailed",
            ]
        );
        assert!(
            out.iter()
                .all(|message| message["sessionId"] == "SID-worker")
        );
        assert_eq!(
            out[0]["params"]["requestId"],
            json!("TID-service-worker.sw-fetch.101.4")
        );
        assert_eq!(out[0]["params"]["request"]["method"], json!("POST"));
        assert_eq!(out[0]["params"]["request"]["postData"], json!("hello"));
        assert_eq!(out[0]["params"]["type"], json!("Fetch"));
        assert_eq!(out[1]["params"]["response"]["status"], json!(201));
        assert_eq!(
            out[1]["params"]["response"]["fromServiceWorker"],
            json!(true)
        );
        assert_eq!(out[2]["params"]["dataLength"], json!(2));
        assert_eq!(out[2]["params"]["encodedDataLength"], json!(2));
        assert_eq!(out[3]["params"]["encodedDataLength"], json!(2));
        assert_eq!(
            out[4]["params"]["requestId"],
            json!("TID-service-worker.sw-fetch.102.5")
        );
        assert_eq!(out[4]["params"]["type"], json!("Script"));
        assert_eq!(
            out[5]["params"]["errorText"],
            json!("ServiceWorkerFallback")
        );
        assert_eq!(out[6]["params"]["type"], json!("Image"));
        assert_eq!(out[7]["params"]["errorText"], json!("network down"));
        assert_eq!(
            out[7]["params"]["__moliServiceWorkerFetchResult"],
            json!("failure")
        );
        assert_eq!(
            out[2]["params"]["__moliServiceWorkerFetchResult"],
            json!("response")
        );
        assert_eq!(
            out[3]["params"]["__moliServiceWorkerFetchResult"],
            json!("response")
        );
        assert!(matches!(
            sidecars[0].as_ref(),
            Some(crate::devtools_runtime::AutomationEvent::NetworkBeforeRequestSent(event))
                if event.request_id.as_str() == "TID-service-worker.sw-fetch.101.4"
                    && event.method.as_deref() == Some("POST")
                    && event.resource_type
                        == Some(crate::devtools_runtime::DevToolsNetworkResourceType::Fetch)
        ));
        assert!(matches!(
            sidecars[1].as_ref(),
            Some(crate::devtools_runtime::AutomationEvent::NetworkResponseStarted(event))
                if event.status == Some(201)
                    && event.url == "https://example.test/api"
        ));
        assert!(matches!(
            sidecars[3].as_ref(),
            Some(crate::devtools_runtime::AutomationEvent::NetworkResponseCompleted(event))
                if event.encoded_data_length == Some(2)
                    && event.resource_type
                        == Some(crate::devtools_runtime::DevToolsNetworkResourceType::Fetch)
        ));
        assert!(matches!(
            sidecars[5].as_ref(),
            Some(crate::devtools_runtime::AutomationEvent::NetworkFetchError(event))
                if event.error_text.as_deref() == Some("ServiceWorkerFallback")
                    && event.resource_type
                        == Some(crate::devtools_runtime::DevToolsNetworkResourceType::Script)
        ));
        assert!(matches!(
            sidecars[7].as_ref(),
            Some(crate::devtools_runtime::AutomationEvent::NetworkFetchError(event))
                if event.error_text.as_deref() == Some("network down")
                    && event.resource_type
                        == Some(crate::devtools_runtime::DevToolsNetworkResourceType::Image)
        ));
    }

    #[test]
    fn service_worker_fetch_diagnostic_aborted_failure_sets_loading_failed_canceled() {
        let diagnostics = vec![RendererServiceWorkerFetchDiagnostic {
            internal_id: 104,
            document_url: "https://example.test/app/".to_owned(),
            request_url: "https://example.test/abort".to_owned(),
            method: "GET".to_owned(),
            request_headers: Vec::new(),
            request_body: None,
            destination: "".to_owned(),
            result: RendererServiceWorkerFetchDiagnosticResult::Failure {
                message: "net::ERR_ABORTED".to_owned(),
            },
        }];
        let events = service_worker_fetch_diagnostic_events(
            "SID-worker",
            "TID-service-worker",
            &diagnostics,
            0,
        );
        let mut out = Vec::new();
        let mut sidecars = Vec::new();
        for event in events {
            let (message, sidecar) = event.into_parts();
            out.push(message);
            sidecars.push(sidecar);
        }

        assert_eq!(out.len(), 2);
        assert_eq!(out[1]["method"], json!("Network.loadingFailed"));
        assert_eq!(out[1]["params"]["errorText"], json!("net::ERR_ABORTED"));
        assert_eq!(out[1]["params"]["canceled"], json!(true));
        assert_eq!(
            out[1]["params"]["__moliServiceWorkerFetchResult"],
            json!("failure")
        );
        assert!(matches!(
            sidecars[1].as_ref(),
            Some(crate::devtools_runtime::AutomationEvent::NetworkFetchError(event))
                if event.error_text.as_deref() == Some("net::ERR_ABORTED")
        ));
    }

    #[test]
    fn service_worker_runtime_inspector_messages_bind_and_prepare_the_exact_run() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-1".to_owned());
        let mut target = ServiceWorkerTargetState::new(
            41,
            23,
            "TID-service-worker".to_owned(),
            "https://example.test/service-worker.js".to_owned(),
            "https://example.test/".to_owned(),
            RendererServiceWorkerVersionStatus::Activated,
            None,
        );
        target.attach_session("SID-first".to_owned());
        target.attach_session("SID-second".to_owned());
        browser_context.insert_service_worker_target(target);
        conn.browser_context = Some(browser_context);

        let messages = runtime_inspector_messages(vec![json!({
            "method": "Runtime.executionContextCreated",
            "params": {
                "context": {
                    "id": 9001,
                    "auxData": { "type": "worker" }
                }
            }
        })]);
        let renderer_run = renderer_run();
        let outputs = record_service_worker_target_runtime_inspector_messages(
            &mut conn,
            "BID-1",
            23,
            renderer_run,
            Some("SID-second".to_owned()),
            messages.clone(),
        )
        .worker_target_lifecycle_outputs;

        assert_eq!(outputs.len(), 1);
        let WorkerTargetLifecycleOutput::ServiceWorkerRuntimeInspectorMessages {
            runtime,
            background_events,
            response_events,
            pending_runtime_console,
            pending_runtime_exceptions,
        } = &outputs[0]
        else {
            panic!("inspector output must retain its exact ServiceWorker run");
        };
        assert_eq!(runtime.target_id(), "TID-service-worker");
        assert_eq!(runtime.session_id(), "SID-second");
        assert!(response_events.is_empty());
        assert!(pending_runtime_console.is_none());
        assert!(pending_runtime_exceptions.is_none());
        assert!(
            background_events.iter().any(|event| {
                event.protocol_method() == Some("Runtime.executionContextCreated")
            })
        );
    }

    #[test]
    fn shared_worker_target_destruction_detaches_session_before_destroyed() {
        let mut conn = CdpConnection::default();
        conn.auto_attach = true;
        conn.set_root_target_discovery_enabled(true);
        conn.browser_context = Some(BrowserContext::new("BID-1".to_owned()));

        let outputs =
            register_shared_worker_target(&mut conn, "BID-1", None, shared_worker_info(13))
                .worker_target_lifecycle_outputs;
        let (target_id, attached_session_id) = outputs
            .iter()
            .find_map(|output| {
                let (attachment, _) = shared_worker_attached_output(output)?;
                Some((
                    attachment.target_id().to_owned(),
                    attachment.session_id().to_owned(),
                ))
            })
            .expect("auto-attached target id");
        let outputs =
            remove_shared_worker_target(&mut conn, "BID-1", SharedWorkerInstanceId::from_u64(13))
                .worker_target_lifecycle_outputs;

        let (retirement, cleanup_plan) = shared_worker_detached_output(&outputs[0])
            .expect("target destruction must transfer its exact attachment to detach");
        assert_eq!(retirement.identity().target_id(), target_id);
        assert_eq!(retirement.identity().session_id(), attached_session_id);
        assert_eq!(cleanup_plan.target_id(), target_id);
        assert_eq!(cleanup_plan.session_id(), attached_session_id);
        assert!(retirement.is_current());
        assert_eq!(
            destroyed_output_target_id(&outputs[1]),
            Some(target_id.as_str())
        );
        assert_eq!(outputs.len(), 2);
        assert!(
            conn.browser_context
                .as_ref()
                .unwrap()
                .target_info(&target_id)
                .is_none()
        );
    }

    #[tokio::test]
    async fn shared_worker_target_destruction_terminates_all_renderer_calls() {
        let mut conn = CdpConnection::default();
        conn.auto_attach = true;
        conn.set_root_target_discovery_enabled(true);
        conn.browser_context = Some(BrowserContext::new("BID-1".to_owned()));

        let outputs =
            register_shared_worker_target(&mut conn, "BID-1", None, shared_worker_info(15))
                .worker_target_lifecycle_outputs;
        let (target_id, attached_session_id) = outputs
            .iter()
            .find_map(|output| {
                let (attachment, _) = shared_worker_attached_output(output)?;
                Some((
                    attachment.target_id().to_owned(),
                    attachment.session_id().to_owned(),
                ))
            })
            .expect("auto-attached target id");
        let target = conn
            .shared_worker_target_for_session_mut(Some(&attached_session_id))
            .expect("shared worker target should be attached");
        target.register_pending_inspector_await(
            &attached_session_id,
            901,
            Some(&attached_session_id),
            None,
        );
        let prepared = target
            .try_register_renderer_call(
                &attached_session_id,
                902,
                Some(moli_page_types::RendererAgentAttachmentId::allocate()),
                crate::conn::RendererCommandDescriptor::from_synthesized_payload(
                    json!({
                        "id": 902,
                        "method": "Console.clearMessages",
                        "sessionId": attached_session_id,
                        "params": {},
                    })
                    .to_string(),
                )
                .expect("supported renderer command"),
            )
            .expect("attached shared worker session")
            .expect("non-await renderer call should register");
        let (correlation, old_sender, terminal_receiver) = prepared.into_parts();

        let outputs =
            remove_shared_worker_target(&mut conn, "BID-1", SharedWorkerInstanceId::from_u64(15))
                .worker_target_lifecycle_outputs;

        let failures = match &outputs[0] {
            WorkerTargetLifecycleOutput::SharedWorkerAttachmentEvents { attachment, events } => {
                assert_eq!(attachment.target_id(), target_id);
                assert_eq!(attachment.session_id(), attached_session_id);
                events
                    .iter()
                    .cloned()
                    .map(BackgroundProtocolEvent::into_protocol_message)
                    .collect::<Vec<_>>()
            }
            _ => panic!("target destruction should fail renderer calls before detach"),
        };
        let await_failure = failures
            .iter()
            .find(|message| message["id"] == json!(901))
            .expect("pending await terminal response");
        assert_eq!(await_failure["id"], json!(901));
        assert_eq!(await_failure["sessionId"], attached_session_id);
        assert_eq!(await_failure["error"]["message"], json!("Target closed"));
        let non_await_failure = failures
            .iter()
            .find(|message| message["id"] == json!(902))
            .expect("non-await terminal response");
        assert_eq!(non_await_failure["sessionId"], attached_session_id);
        assert_eq!(non_await_failure["error"]["code"], json!(-32000));
        assert_eq!(
            non_await_failure["error"]["message"],
            json!("Target closed")
        );
        assert!(
            old_sender
                .send(json!({
                    "id": correlation.renderer_call_id().get(),
                    "result": {},
                }))
                .is_err(),
            "target destruction must invalidate the renderer's old response lease"
        );
        let terminal = terminal_receiver
            .await
            .expect("terminal transition should complete the response receiver");
        assert_eq!(terminal.renderer_agent_attachment_id(), None);
        assert_eq!(
            terminal
                .output
                .protocol_response(terminal.call_id)
                .expect("terminal renderer response")["error"]["message"],
            json!("Target closed")
        );
        let (retirement, cleanup_plan) = shared_worker_detached_output(&outputs[1])
            .expect("target destruction must transfer its exact attachment to detach");
        assert_eq!(retirement.identity().target_id(), target_id);
        assert_eq!(retirement.identity().session_id(), attached_session_id);
        assert_eq!(cleanup_plan.target_id(), target_id);
        assert_eq!(cleanup_plan.session_id(), attached_session_id);
        assert_eq!(
            destroyed_output_target_id(&outputs[2]),
            Some(target_id.as_str())
        );
        assert_eq!(outputs.len(), 3);
    }

    #[test]
    fn late_shared_worker_messages_after_destroy_are_dropped_without_page_fallback() {
        let mut conn = CdpConnection::default();
        conn.auto_attach = true;
        conn.set_root_target_discovery_enabled(true);
        let mut browser_context = BrowserContext::new("BID-1".to_owned());
        browser_context.set_active_target_id("TID-page");
        browser_context.attach_active_session("SID-page");
        conn.browser_context = Some(browser_context);

        let outputs =
            register_shared_worker_target(&mut conn, "BID-1", None, shared_worker_info(17))
                .worker_target_lifecycle_outputs;
        let (target_id, attached_session_id) = outputs
            .iter()
            .find_map(|output| {
                let (attachment, _) = shared_worker_attached_output(output)?;
                Some((
                    attachment.target_id().to_owned(),
                    attachment.session_id().to_owned(),
                ))
            })
            .expect("auto-attached target id");

        let destroy_outputs =
            remove_shared_worker_target(&mut conn, "BID-1", SharedWorkerInstanceId::from_u64(17))
                .worker_target_lifecycle_outputs;
        assert!(
            destroy_outputs
                .iter()
                .any(|output| destroyed_output_target_id(output) == Some(target_id.as_str()))
        );

        let late_outputs = record_shared_worker_target_runtime_inspector_messages(
            &mut conn,
            "BID-1",
            SharedWorkerInstanceId::from_u64(17),
            None,
            vec![RendererRuntimeInspectorMessage::protocol(json!({
                "id": 902,
                "result": {
                    "result": {
                        "type": "string",
                        "value": "late"
                    }
                }
            }))],
        );

        assert!(
            late_outputs.worker_target_lifecycle_outputs.is_empty(),
            "late inspector replies for a destroyed shared worker target must be dropped"
        );
        let late_console_outputs = record_shared_worker_target_console_message(
            &mut conn,
            "BID-1",
            SharedWorkerInstanceId::from_u64(17),
            RendererSharedWorkerConsoleMessage {
                message: "warn: late shared worker console".to_owned(),
                args: Vec::new(),
                stack: None,
            },
        );
        assert!(
            late_console_outputs
                .worker_target_lifecycle_outputs
                .is_empty(),
            "late console output for a destroyed shared worker target must be dropped"
        );
        assert_eq!(conn.session_route(Some(&attached_session_id)), None);
        assert_eq!(
            conn.session_route(Some("SID-page")),
            Some(crate::conn::CdpSessionRoute::ActiveTarget {
                browser_context_id: "BID-1".to_owned(),
                target_id: Some("TID-page".to_owned())
            }),
            "dropping late shared worker output must not disturb or reuse the active page session"
        );
        assert!(
            conn.browser_context
                .as_ref()
                .unwrap()
                .shared_worker_target(&target_id)
                .is_none()
        );
        assert!(
            conn.browser_context
                .as_ref()
                .unwrap()
                .target_infos()
                .into_iter()
                .all(|info| info["type"] != json!("shared_worker")),
            "late shared worker output must not recreate target state after destroy"
        );
    }

    #[tokio::test]
    async fn shared_worker_target_console_messages_emit_to_target_session_and_advance_cursors() {
        let mut conn = CdpConnection::default();
        conn.browser_context = Some(BrowserContext::new("BID-1".to_owned()));
        let mut target = SharedWorkerTargetState::new(
            moli_core::RendererOwnerLocalHostId::new_for_testing(1),
            SharedWorkerInstanceId::from_u64(17),
            "TID-shared-worker".to_owned(),
            None,
            "https://example.test/shared-worker.js".to_owned(),
            "worker".to_owned(),
        );
        target.attach_session("SID-shared-worker".to_owned());
        target.set_console_enabled("SID-shared-worker", true);
        target.set_runtime_frontend_enabled("SID-shared-worker", true);
        target.record_runtime_execution_context_created_event(&worker_context_created_event(
            90_017, "worker",
        ));
        conn.browser_context
            .as_mut()
            .unwrap()
            .insert_shared_worker_target(target);

        let outputs = record_shared_worker_target_console_message(
            &mut conn,
            "BID-1",
            SharedWorkerInstanceId::from_u64(17),
            RendererSharedWorkerConsoleMessage {
                message: "warn: from shared worker".to_owned(),
                args: Vec::new(),
                stack: None,
            },
        );
        let mut prepared_outputs =
            ProtocolOutputPayloads::from_slot(TargetPreparedOutputSlot::from_outputs(outputs));
        let mut command_context = crate::conn::CommandDispatchContext::default();
        emit_target_lifecycle_events(
            &mut conn,
            &mut ProtocolOutputProjectionContext::new(None, &mut command_context),
            Some(&mut prepared_outputs),
        )
        .await;
        let events = command_context.take_protocol_events();

        let (console_event, console_sidecar) = events
            .iter()
            .find_map(|event| {
                let (message, sidecar) = event.clone().into_parts();
                (message["method"] == json!("Console.messageAdded")).then_some((message, sidecar))
            })
            .expect("Console.messageAdded should be emitted for enabled shared worker target");
        assert_eq!(console_event["sessionId"], "SID-shared-worker");
        assert_eq!(
            console_event["params"]["message"]["text"],
            "from shared worker"
        );
        assert!(matches!(
            console_sidecar,
            Some(crate::devtools_runtime::AutomationEvent::RuntimeConsoleApiCalled(event))
                if event.console_type == "warning"
                    && event.text == "from shared worker"
                    && event.execution_context_id.is_none()
        ));
        let (runtime_event, runtime_sidecar) = events
            .iter()
            .find_map(|event| {
                (event.protocol_message().is_none()
                    && event.protocol_method() == Some("Runtime.consoleAPICalled"))
                .then(|| event.clone().into_parts())
            })
            .expect("Runtime.consoleAPICalled should be emitted for enabled shared worker target");
        assert_eq!(runtime_event["sessionId"], "SID-shared-worker");
        assert_eq!(runtime_event["params"]["type"], "warning");
        assert_eq!(runtime_event["params"]["executionContextId"], json!(90_017));
        assert_eq!(
            runtime_event["params"]["args"],
            json!([{
                "type": "string",
                "value": "from shared worker"
            }])
        );
        assert!(matches!(
            runtime_sidecar,
            Some(crate::devtools_runtime::AutomationEvent::RuntimeConsoleApiCalled(event))
                if event.console_type == "warning"
                    && event.text == "from shared worker"
                    && event.execution_context_id == Some(90_017)
        ));

        let target = conn
            .browser_context
            .as_ref()
            .unwrap()
            .shared_worker_target("TID-shared-worker")
            .unwrap();
        assert!(
            target
                .pending_console_domain_messages("SID-shared-worker")
                .is_empty()
        );
        assert!(
            target
                .pending_runtime_console_messages("SID-shared-worker")
                .is_empty()
        );
    }
}

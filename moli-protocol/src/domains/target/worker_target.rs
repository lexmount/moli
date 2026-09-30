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

use crate::automation::{DevToolsNetworkResourceType, RuntimeExecutionContextsClearedEvent};
#[cfg(test)]
use crate::automation::{DevToolsTargetInfo, DevToolsTargetKind, RuntimeExecutionContextEvent};
use crate::{
    conn::{
        BackgroundProtocolEvent, CdpConnection, CdpSessionRoute, CommandOwnerScope,
        PreparedTargetAttach, PreparedTargetHostDelta, RendererPageResidenceIdentity,
        ServiceWorkerRuntimeExceptionSnapshot, ServiceWorkerTargetState, SharedWorkerTargetState,
        TargetAttachSessionCommit, TargetPageResidenceIdentity,
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

mod lifecycle_projection;
use lifecycle_projection::*;
mod dedicated_worker;
use dedicated_worker::*;
mod shared_worker;
use shared_worker::*;
mod service_worker_targets;
use service_worker_targets::*;
mod event_projection;
pub(super) use dedicated_worker::dedicated_worker_auto_attach_owner_session_allowed;
pub(in crate::domains) use dedicated_worker::{
    dedicated_worker_main_script_network_replay_for_session,
    release_failed_dedicated_worker_target_after_debugger_resume,
    retire_dedicated_worker_targets_for_replaced_page_async,
};
pub(in crate::domains) use event_projection::project_worker_target_output_async;
use event_projection::*;
pub(in crate::domains) use lifecycle_projection::{
    dedicated_worker_target_lifecycle_prepared_outputs_for_event,
    service_worker_target_lifecycle_prepared_outputs_for_event,
    shared_worker_target_lifecycle_prepared_outputs_for_event,
};
pub(super) use service_worker_targets::close_browser_context_worker_targets_for_dispose_async;
pub(super) use shared_worker::{
    close_dedicated_worker_target_for_target_close_async,
    close_shared_worker_target_for_target_close_async,
};

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

#[cfg(test)]
#[path = "worker_target_attachment_tests.rs"]
mod worker_target_attachment_tests;

#[cfg(test)]
mod tests;

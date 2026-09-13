use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource};

use crate::{
    native_bridge::WindowExecutionContextIdentity,
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
    types::{DedicatedWorkerId, SubresourcePolicyContext},
    worker::{
        WorkerErrorPhase, WorkerErrorSource, WorkerParentErrorEventKind, WorkerScriptKind,
        WorkerScriptSource,
    },
};

use super::{RendererOwnerWakeSender, RendererOwnerWakeSource, RendererPageTaskReadySignal};

/// Exact Page-side Worker wrapper that owns one client-facing event.
///
/// DedicatedWorker ids are only unique inside one PageVm. The root token
/// prevents a late event from crossing PageVm replacement, while the Window
/// identity prevents an old realm from dispatching through a replacement
/// wrapper in the same PageVm.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageDedicatedWorkerClientEventOwner {
    root_document: RendererDocumentToken,
    execution_context: WindowExecutionContextIdentity,
    worker_id: DedicatedWorkerId,
}

impl RendererPageDedicatedWorkerClientEventOwner {
    pub(crate) const fn new(
        root_document: RendererDocumentToken,
        execution_context: WindowExecutionContextIdentity,
        worker_id: DedicatedWorkerId,
    ) -> Self {
        Self {
            root_document,
            execution_context,
            worker_id,
        }
    }

    #[cfg(test)]
    pub(crate) const fn root_document(self) -> RendererDocumentToken {
        self.root_document
    }

    pub(crate) const fn execution_context(self) -> WindowExecutionContextIdentity {
        self.execution_context
    }

    pub(crate) const fn worker_id(self) -> DedicatedWorkerId {
        self.worker_id
    }
}

/// A user-visible message produced by one DedicatedWorker.
///
/// Resource, console, inspector and WebSocket records deliberately cannot be
/// represented here. They remain on the Worker host bridge until
/// their own P3 source is migrated.
#[derive(Debug)]
pub(crate) enum RendererDedicatedWorkerMessageEvent {
    Message(crate::structured_clone::V8StructuredClonePayload),
    Error {
        message: String,
        filename: String,
        lineno: u32,
        colno: u32,
        event_kind: WorkerParentErrorEventKind,
        phase: WorkerErrorPhase,
        source: WorkerErrorSource,
    },
}

/// One owner-side task emitted by a DedicatedWorker relay.
///
/// Message/error variants can dispatch client callbacks. Load and terminal
/// variants update wrapper state without pretending that a callback ran.
///
/// The target identity is absent by construction: it lives in the bound
/// producer and cannot drift from this payload.
#[derive(Debug)]
pub(crate) enum RendererDedicatedWorkerClientEvent {
    ScriptLoaded {
        script_url: String,
        script_source: WorkerScriptSource,
        script_kind: WorkerScriptKind,
        secure_context: bool,
        response_referrer_policy: Option<String>,
        network_partition_key: Option<String>,
        policy_context: SubresourcePolicyContext,
        content_security_policies: Vec<String>,
        content_security_report_only_policies: Vec<String>,
        content_security_reporting_endpoints:
            crate::content_security_policy::ContentSecurityPolicyReportingEndpoints,
    },
    ScriptLoadFailed {
        script_url: String,
        error_message: String,
    },
    Message(RendererDedicatedWorkerMessageEvent),
    /// Relay terminal ordered behind all client-source records.
    ClientSourceDrained,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererDedicatedWorkerClientEventKind {
    ScriptLoaded,
    ScriptLoadFailed,
    Message,
    Error,
    ClientSourceDrained,
}

impl RendererDedicatedWorkerClientEvent {
    const fn kind(&self) -> RendererDedicatedWorkerClientEventKind {
        match self {
            Self::ScriptLoaded { .. } => RendererDedicatedWorkerClientEventKind::ScriptLoaded,
            Self::ScriptLoadFailed { .. } => {
                RendererDedicatedWorkerClientEventKind::ScriptLoadFailed
            }
            Self::Message(RendererDedicatedWorkerMessageEvent::Message(_)) => {
                RendererDedicatedWorkerClientEventKind::Message
            }
            Self::Message(RendererDedicatedWorkerMessageEvent::Error { .. }) => {
                RendererDedicatedWorkerClientEventKind::Error
            }
            Self::ClientSourceDrained => {
                RendererDedicatedWorkerClientEventKind::ClientSourceDrained
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct RendererPageDedicatedWorkerClientEventTask {
    owner: RendererPageDedicatedWorkerClientEventOwner,
    event: RendererDedicatedWorkerClientEvent,
}

impl RendererPageDedicatedWorkerClientEventTask {
    fn new(
        owner: RendererPageDedicatedWorkerClientEventOwner,
        event: RendererDedicatedWorkerClientEvent,
    ) -> Self {
        Self { owner, event }
    }

    pub(crate) const fn owner(&self) -> RendererPageDedicatedWorkerClientEventOwner {
        self.owner
    }

    pub(crate) const fn event_kind(&self) -> RendererDedicatedWorkerClientEventKind {
        self.event.kind()
    }

    pub(crate) fn into_event(self) -> RendererDedicatedWorkerClientEvent {
        self.event
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageDedicatedWorkerClientEventRouteClosed;

#[derive(Clone, Debug)]
pub(crate) struct RendererPageDedicatedWorkerClientEventRoute {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageDedicatedWorkerClientEventTask>,
        RendererPageTaskReadySignal,
    >,
    page_token: crate::runtime::RendererPageToken,
}

impl RendererPageDedicatedWorkerClientEventRoute {
    pub(crate) fn sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageDedicatedWorkerClientEventSender {
        RendererPageDedicatedWorkerClientEventSender {
            task_route: self.task_route.clone(),
            root_document,
            page_token: self.page_token,
        }
    }

    fn same_route_as(&self, source: &RendererPageDedicatedWorkerClientEventSource) -> bool {
        self.task_route.same_source_as(&source.source)
    }
}

/// PageVm-stamped route used only to bind a concrete Worker wrapper.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageDedicatedWorkerClientEventSender {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageDedicatedWorkerClientEventTask>,
        RendererPageTaskReadySignal,
    >,
    root_document: RendererDocumentToken,
    page_token: crate::runtime::RendererPageToken,
}

impl RendererPageDedicatedWorkerClientEventSender {
    pub(crate) const fn page_token(&self) -> crate::runtime::RendererPageToken {
        self.page_token
    }

    pub(crate) fn bind_worker(
        &self,
        execution_context: WindowExecutionContextIdentity,
        worker_id: DedicatedWorkerId,
    ) -> RendererPageDedicatedWorkerClientEventProducer {
        RendererPageDedicatedWorkerClientEventProducer {
            task_route: self.task_route.clone(),
            owner: RendererPageDedicatedWorkerClientEventOwner::new(
                self.root_document,
                execution_context,
                worker_id,
            ),
        }
    }
}

/// Exact owner capability retained for the lifetime of one Worker wrapper.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageDedicatedWorkerClientEventProducer {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageDedicatedWorkerClientEventTask>,
        RendererPageTaskReadySignal,
    >,
    owner: RendererPageDedicatedWorkerClientEventOwner,
}

impl RendererPageDedicatedWorkerClientEventProducer {
    pub(crate) fn send(
        &self,
        event: RendererDedicatedWorkerClientEvent,
    ) -> Result<(), RendererPageDedicatedWorkerClientEventRouteClosed> {
        self.task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(
                RendererPageDedicatedWorkerClientEventTask::new(self.owner, event),
            ))
            .map_err(|_| RendererPageDedicatedWorkerClientEventRouteClosed)
    }

    pub(crate) const fn owner(&self) -> RendererPageDedicatedWorkerClientEventOwner {
        self.owner
    }
}

/// Unique Page-lifetime consumer for DedicatedWorker client events.
#[derive(Debug)]
pub(crate) struct RendererPageDedicatedWorkerClientEventSource {
    source: OwnerReadyTaskSource<
        ReadyPageTask<RendererPageDedicatedWorkerClientEventTask>,
        RendererPageTaskReadySignal,
    >,
    page_token: crate::runtime::RendererPageToken,
}

impl RendererPageDedicatedWorkerClientEventSource {
    pub(crate) fn new(owner_wake: RendererOwnerWakeSender) -> Self {
        let page_token = owner_wake.token();
        Self {
            source: OwnerReadyTaskSource::new(RendererPageTaskReadySignal::new(
                owner_wake,
                RendererOwnerWakeSource::DedicatedWorkerClientEvent,
            )),
            page_token,
        }
    }

    pub(crate) fn route(&self) -> RendererPageDedicatedWorkerClientEventRoute {
        RendererPageDedicatedWorkerClientEventRoute {
            task_route: self.source.route(),
            page_token: self.page_token,
        }
    }

    pub(crate) fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.front().map(ReadyPageTask::metadata)
    }

    pub(crate) fn next_ready_owner(
        &mut self,
    ) -> Option<RendererPageDedicatedWorkerClientEventOwner> {
        self.source.front().map(|ready| ready.value().owner())
    }

    pub(crate) fn pop_front(
        &mut self,
    ) -> Option<(
        RendererPageTaskReadyMetadata,
        RendererPageDedicatedWorkerClientEventTask,
    )> {
        self.source.pop_front().map(ReadyPageTask::into_parts)
    }

    pub(crate) fn has_ready_task(&mut self) -> bool {
        !self.source.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.source.clear_local();
    }

    pub(crate) fn route_matches(
        &self,
        route: &RendererPageDedicatedWorkerClientEventRoute,
    ) -> bool {
        route.same_route_as(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageDedicatedWorkerClientEventTargetEffect {
    /// The selected event changed the exact current Worker's non-callback
    /// state. It still owns an ordinary task-end checkpoint.
    StateAppliedToCurrentOwner,
    /// The selected event dispatched callback-visible work in the exact
    /// current Worker/Window realm.
    CallbackDispatchedToCurrentOwner,
    /// The exact target was current and the event was consumed, but no
    /// callback matched it.
    CurrentOwnerHadNoCallback,
    /// The target passed Page arbitration but disappeared before body
    /// application. The selected current task still owns its checkpoint.
    CurrentOwnerLostDuringExecution,
    /// The queued event belonged to a retired root Document or Window realm
    /// and was discarded without entering V8.
    DiscardedStaleOwner {
        current_owner: Option<RendererPageDedicatedWorkerClientEventOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageDedicatedWorkerClientEventTurnAction {
    pub(crate) owner: RendererPageDedicatedWorkerClientEventOwner,
    pub(crate) event_kind: RendererDedicatedWorkerClientEventKind,
    pub(crate) target_effect: PageDedicatedWorkerClientEventTargetEffect,
}

pub(crate) type PageDedicatedWorkerClientEventTurnOutcome =
    PageOwnerTurnOutcome<PageDedicatedWorkerClientEventTurnAction>;

use crate::protocol_types::{
    ChildFrameDocumentNetworkActivitySnapshot, ChildFrameDocumentOpenedSnapshot,
    ChildFrameNavigationSnapshot, ChildFrameTreeEventSnapshot,
};
use crate::runtime::{
    DetachedParserScriptFetchContinuation, RendererDedicatedWorkerObservation,
    RendererDocumentLifecycleEvent, RendererDocumentLifecycleIdentity,
    RendererDocumentSourcedSameDocumentNavigation,
    RendererDocumentSourcedTopLevelLocationNavigation, RendererDomMutationEventBatch,
    RendererJavaScriptDialogOpening, RendererMainDocumentCommit, RendererPendingDownloadActivation,
    RendererPendingFileChooserActivation, RendererPendingTopLevelHistoryTraversal,
    RendererPopupOpening, RendererRuntimeCommandCausalIdentity,
    RendererRuntimeInspectorMessageBatch, RendererServiceWorkerTargetEvent,
    RendererSharedWorkerObservation,
};
use moli_page_types::{
    PendingRuntimeBindingCall, PendingSubresourceContinueEvent, PendingSubresourceFetchInfo,
    ScriptNetworkOutputItem,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererDocumentTitleChanged {
    pub source_document: RendererDocumentLifecycleIdentity,
    pub title: String,
}

/// A renderer-neutral browser-owner action.
///
/// Variants move here as their old pending queue is removed. Keeping actions
/// separate from observations makes listener-independent progress explicit
/// without changing their position in the enclosing FIFO.
#[derive(Clone, Debug, PartialEq)]
pub enum RendererOwnerAction {
    FileChooser(RendererPendingFileChooserActivation),
    Download(RendererPendingDownloadActivation),
    ChildFrameTree {
        source_document: RendererDocumentLifecycleIdentity,
        event: ChildFrameTreeEventSnapshot,
    },
    ChildFrameDocumentOpened {
        source_document: RendererDocumentLifecycleIdentity,
        event: ChildFrameDocumentOpenedSnapshot,
    },
    ChildFrameDocumentNetwork {
        source_document: RendererDocumentLifecycleIdentity,
        event: ChildFrameDocumentNetworkActivitySnapshot,
    },
    ChildFrameLoad {
        source_document: RendererDocumentLifecycleIdentity,
        event: ChildFrameNavigationSnapshot,
    },
    SameDocumentNavigation(RendererDocumentSourcedSameDocumentNavigation),
    TopLevelLocationNavigation(RendererDocumentSourcedTopLevelLocationNavigation),
    TopLevelHistoryTraversal(RendererPendingTopLevelHistoryTraversal),
    SubresourceFetchPause {
        source_document: RendererDocumentLifecycleIdentity,
        info: Box<PendingSubresourceFetchInfo>,
    },
    SubresourceContinue {
        source_document: RendererDocumentLifecycleIdentity,
        event: Box<PendingSubresourceContinueEvent>,
    },
    DetachedParserScriptFetchPause {
        source_document: RendererDocumentLifecycleIdentity,
        info: Box<PendingSubresourceFetchInfo>,
        continuation: DetachedParserScriptFetchContinuation,
    },
    ServiceWorkerTargetLifecycle(RendererServiceWorkerTargetEvent),
}

/// A concrete renderer fact that protocol may project to interested sessions.
///
/// These are semantic payloads, not CDP JSON. Source identities are already
/// frozen inside the domain payload; target/session authorization remains a
/// protocol-boundary responsibility.
#[derive(Clone, Debug, PartialEq)]
pub enum RendererProtocolObservation {
    WorkerLifecycle(crate::runtime::RendererWorkerLifecycleObservation),
    SharedWorker(RendererSharedWorkerObservation),
    DedicatedWorker(RendererDedicatedWorkerObservation),
    Popup(std::sync::Arc<RendererPopupOpening>),
    JavaScriptDialog(std::sync::Arc<RendererJavaScriptDialogOpening>),
    MainDocumentCommit(RendererMainDocumentCommit),
    DocumentTitleChanged(RendererDocumentTitleChanged),
    DocumentLifecycle(RendererDocumentLifecycleEvent),
    Network {
        source_document: RendererDocumentLifecycleIdentity,
        item: ScriptNetworkOutputItem,
    },
    RuntimeBinding(PendingRuntimeBindingCall),
    DomMutations(RendererDomMutationEventBatch),
    RuntimeInspector(RendererRuntimeInspectorMessageBatch),
    RuntimeConsole(crate::runtime::RuntimeConsoleMessageSnapshot),
    InspectorIssue {
        source_document: RendererDocumentLifecycleIdentity,
        issue: moli_page_types::InspectorIssueSnapshot,
    },
    WindowOpen(crate::runtime::RendererPendingWindowOpenEvent),
    RuntimeLifecycleError {
        text: String,
        execution_context_id: Option<i64>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum RendererOutputItem {
    OwnerAction(RendererOwnerAction),
    Observation(RendererProtocolObservation),
}

/// One record in exact producer append order.
///
/// Runtime-command causality is record-local. A checkpoint may run unrelated
/// microtasks in the same physical owner turn, and those records must not be
/// attributed to that Runtime command merely because they share a
/// publication. The command response still waits for the whole concrete
/// publication: like Chromium's per-session notification flush, every fact
/// already produced before the response keeps its FIFO position.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PendingRendererOutputRecord {
    causal_command: Option<RendererRuntimeCommandCausalIdentity>,
    item: RendererOutputItem,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererOutputResolutionError {
    RuntimeInspector,
    SharedWorkerRuntimeInspector,
    ServiceWorkerRuntimeInspector,
    DedicatedWorkerRuntimeInspector,
}

impl PendingRendererOutputRecord {
    pub(crate) fn from_parts(
        causal_command: Option<RendererRuntimeCommandCausalIdentity>,
        item: RendererOutputItem,
    ) -> Self {
        Self {
            causal_command,
            item,
        }
    }

    pub(crate) fn observation(
        causal_command: Option<RendererRuntimeCommandCausalIdentity>,
        observation: RendererProtocolObservation,
    ) -> Self {
        Self {
            causal_command,
            item: RendererOutputItem::Observation(observation),
        }
    }

    pub(crate) fn owner_action(
        causal_command: Option<RendererRuntimeCommandCausalIdentity>,
        action: RendererOwnerAction,
    ) -> Self {
        Self {
            causal_command,
            item: RendererOutputItem::OwnerAction(action),
        }
    }

    #[cfg(test)]
    pub(crate) fn item(&self) -> &RendererOutputItem {
        &self.item
    }

    fn resolution_error(&self) -> Option<RendererOutputResolutionError> {
        match &self.item {
            RendererOutputItem::Observation(RendererProtocolObservation::RuntimeInspector(
                batch,
            )) => (!batch.has_resolved_source_identities())
                .then_some(RendererOutputResolutionError::RuntimeInspector),
            RendererOutputItem::Observation(RendererProtocolObservation::SharedWorker(
                crate::runtime::RendererSharedWorkerObservation::RuntimeInspectorMessages {
                    messages,
                    ..
                },
            )) => (!messages.iter().all(
                crate::runtime::RendererRuntimeInspectorMessage::has_resolved_source_identity,
            ))
            .then_some(RendererOutputResolutionError::SharedWorkerRuntimeInspector),
            RendererOutputItem::OwnerAction(RendererOwnerAction::ServiceWorkerTargetLifecycle(
                crate::runtime::RendererServiceWorkerTargetEvent::RuntimeInspectorMessages {
                    messages,
                    ..
                },
            )) => (!messages.iter().all(
                crate::runtime::RendererRuntimeInspectorMessage::has_resolved_source_identity,
            ))
            .then_some(RendererOutputResolutionError::ServiceWorkerRuntimeInspector),
            RendererOutputItem::Observation(RendererProtocolObservation::DedicatedWorker(
                crate::runtime::RendererDedicatedWorkerObservation::RuntimeInspectorMessages {
                    messages,
                    ..
                },
            )) => (!messages.iter().all(
                crate::runtime::RendererRuntimeInspectorMessage::has_resolved_source_identity,
            ))
            .then_some(RendererOutputResolutionError::DedicatedWorkerRuntimeInspector),
            RendererOutputItem::OwnerAction(_) | RendererOutputItem::Observation(_) => None,
        }
    }

    pub(crate) fn with_runtime_inspector_batch_mut(
        &mut self,
        visit: impl FnOnce(&mut RendererRuntimeInspectorMessageBatch),
    ) {
        if let RendererOutputItem::Observation(RendererProtocolObservation::RuntimeInspector(
            batch,
        )) = &mut self.item
        {
            visit(batch);
        }
    }

    /// Converts the producer-side record into the only record type accepted
    /// by a publication. Runtime Inspector records may be enriched between
    /// append and this boundary; unresolved identity can no longer cross it.
    pub(crate) fn resolve(self) -> Result<RendererOutputRecord, RendererOutputResolutionError> {
        if let Some(error) = self.resolution_error() {
            return Err(error);
        }
        Ok(RendererOutputRecord {
            causal_command: self.causal_command,
            item: self.item,
        })
    }
}

/// A protocol-publication record whose source identities are fully frozen.
///
/// There is intentionally no general production constructor. Renderer code
/// must first create [`PendingRendererOutputRecord`] and cross its explicit
/// resolution boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct RendererOutputRecord {
    causal_command: Option<RendererRuntimeCommandCausalIdentity>,
    item: RendererOutputItem,
}

impl RendererOutputRecord {
    pub fn causal_command(&self) -> Option<&RendererRuntimeCommandCausalIdentity> {
        self.causal_command.as_ref()
    }

    pub fn item(&self) -> &RendererOutputItem {
        &self.item
    }

    pub(crate) fn requires_essential_transport_admission(&self) -> bool {
        matches!(
            self.item,
            RendererOutputItem::OwnerAction(_)
                | RendererOutputItem::Observation(
                    RendererProtocolObservation::WorkerLifecycle(_)
                        | RendererProtocolObservation::SharedWorker(_)
                        | RendererProtocolObservation::DedicatedWorker(_)
                )
        )
    }

    pub fn into_parts(
        self,
    ) -> (
        Option<RendererRuntimeCommandCausalIdentity>,
        RendererOutputItem,
    ) {
        (self.causal_command, self.item)
    }

    #[doc(hidden)]
    pub fn new_for_test(item: RendererOutputItem) -> Self {
        PendingRendererOutputRecord {
            causal_command: None,
            item,
        }
        .resolve()
        .unwrap_or_else(|_| panic!("test RendererOutputRecord must carry resolved identities"))
    }
}

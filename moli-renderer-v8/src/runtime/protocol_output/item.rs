use crate::protocol_types::{
    ChildFrameDocumentNetworkActivitySnapshot, ChildFrameDocumentOpenedSnapshot,
    ChildFrameNavigationSnapshot, ChildFrameTreeEventSnapshot,
};
use crate::runtime::{
    DetachedParserScriptFetchContinuation, RendererDedicatedWorkerTargetEvent,
    RendererDocumentLifecycleEvent, RendererDocumentLifecycleIdentity,
    RendererDocumentSourcedSameDocumentNavigation,
    RendererDocumentSourcedTopLevelLocationNavigation, RendererDomMutationEventBatch,
    RendererMainDocumentCommit, RendererPendingDownloadActivation,
    RendererPendingFileChooserActivation, RendererPendingJavaScriptDialog,
    RendererPendingPopupActivation, RendererPendingTopLevelHistoryTraversal,
    RendererRuntimeCommandCausalIdentity, RendererRuntimeInspectorMessageBatch,
    RendererServiceWorkerTargetEvent, RendererSharedWorkerTargetEvent,
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

/// Browser-owner close semantics selected by the producer that completed the
/// renderer-side beforeunload decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererTopLevelCloseSource {
    /// Script-originated `Window.close()`. Protocol must keep the exact Page
    /// currentness check through final teardown.
    Window,
    /// Browser-originated `Page.close`. A replacement Document may become
    /// current while the browser-owned close remains pending.
    Page,
    /// Browser-originated `Target.closeTarget`.
    Target,
}

/// A renderer-neutral browser-owner action.
///
/// Variants move here as their old pending queue is removed. Keeping actions
/// separate from observations makes listener-independent progress explicit
/// without changing their position in the enclosing FIFO.
#[derive(Clone, Debug, PartialEq)]
pub enum RendererOwnerAction {
    /// Requests browser-owner activation of an exact renderer Page. The
    /// action stays in the incumbent turn's stream so a cross-Page call keeps
    /// its response ordering; the payload independently freezes the target.
    /// Renderer admission has already consumed any required incumbent
    /// transient activation (or accepted the opener exemption).
    TopLevelFocus(crate::runtime::RendererResolvedPopupTarget),
    /// Requests browser-owner retirement of this output stream's top-level
    /// browsing context. The Page-scoped lifecycle has already transitioned
    /// to script-visible `Closing` when this record is appended.
    TopLevelClose(RendererTopLevelCloseSource),
    /// Renderer-stream barrier after accepted-close network terminals. The
    /// browser owner may start unload only after this record reaches ingress.
    TopLevelCloseNetworkDrained(RendererTopLevelCloseSource),
    /// Renderer ACK after pagehide/unload for an accepted browser close. This
    /// record is appended after every unload-produced output in the same Page
    /// stream, so protocol teardown needs no recursive command fence.
    TopLevelCloseUnloadAck(RendererTopLevelCloseSource),
    /// Routes an operation accepted through a live RemoteWindowProxy to the
    /// exact target Page. The target renderer revalidates the group-qualified
    /// endpoint before touching its current LocalWindow.
    RemoteWindowProxy(crate::runtime::RendererRemoteWindowProxyCommand),
    FileChooser(RendererPendingFileChooserActivation),
    Download(RendererPendingDownloadActivation),
    JavaScriptDialog(RendererPendingJavaScriptDialog),
    Popup(RendererPendingPopupActivation),
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
    SharedWorkerTargetLifecycle(RendererSharedWorkerTargetEvent),
    ServiceWorkerTargetLifecycle(RendererServiceWorkerTargetEvent),
    DedicatedWorkerTargetLifecycle(RendererDedicatedWorkerTargetEvent),
}

/// A concrete renderer fact that protocol may project to interested sessions.
///
/// These are semantic payloads, not CDP JSON. Source identities are already
/// frozen inside the domain payload; target/session authorization remains a
/// protocol-boundary responsibility.
#[derive(Clone, Debug, PartialEq)]
pub enum RendererProtocolObservation {
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
            RendererOutputItem::OwnerAction(RendererOwnerAction::SharedWorkerTargetLifecycle(
                crate::runtime::RendererSharedWorkerTargetEvent::RuntimeInspectorMessages {
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
            RendererOutputItem::OwnerAction(
                RendererOwnerAction::DedicatedWorkerTargetLifecycle(
                    crate::runtime::RendererDedicatedWorkerTargetEvent::RuntimeInspectorMessages {
                        messages,
                        ..
                    },
                ),
            ) => (!messages.iter().all(
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

    pub(crate) fn is_owner_action(&self) -> bool {
        matches!(self.item, RendererOutputItem::OwnerAction(_))
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

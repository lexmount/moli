use crate::protocol_types::{
    ChildFrameDocumentNetworkActivitySnapshot, ChildFrameDocumentOpenedSnapshot,
    ChildFrameNavigationSnapshot, ChildFrameTreeEventSnapshot,
};
#[cfg(feature = "devtools")]
use crate::runtime::RendererDomMutationEventBatch;
use crate::runtime::{
    DetachedParserScriptFetchContinuation, RendererDedicatedWorkerTargetEvent,
    RendererDocumentLifecycleEvent, RendererDocumentLifecycleIdentity,
    RendererDocumentSourcedSameDocumentNavigation,
    RendererDocumentSourcedTopLevelLocationNavigation, RendererMainDocumentCommit,
    RendererPendingDownloadActivation, RendererPendingFileChooserActivation,
    RendererPendingJavaScriptDialog, RendererPendingPopupActivation,
    RendererPendingTopLevelHistoryTraversal, RendererRuntimeCommandCausalIdentity,
    RendererRuntimeInspectorMessageBatch, RendererServiceWorkerTargetEvent,
    RendererSharedWorkerTargetEvent,
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
    JavaScriptDialog(RendererPendingJavaScriptDialog),
    Popup(RendererPendingPopupActivation),
    CloseAuxiliaryWindow(crate::runtime::RendererAuxiliaryWindow),
    NavigateAuxiliaryWindow {
        window: crate::runtime::RendererAuxiliaryWindow,
        url: String,
        kind: crate::runtime::RendererAuxiliaryNavigationKind,
        document_response: Option<super::super::RendererAuxiliaryDocumentResponse>,
    },
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
    /// The new Document is installed, before its parser can attach child Frames.
    ChildFrameCommit {
        source_document: RendererDocumentLifecycleIdentity,
        event: ChildFrameNavigationSnapshot,
        /// Co-published before commit; stale loads use ChildFrameDocumentNetwork.
        document_network: Option<Box<ChildFrameDocumentNetworkActivitySnapshot>>,
    },
    /// Load delivery finished, including any descendant Document blockers.
    /// This must not be projected as another navigation commit.
    ChildFrameLoad {
        source_document: RendererDocumentLifecycleIdentity,
        event: ChildFrameNavigationSnapshot,
    },
    SameDocumentNavigation(RendererDocumentSourcedSameDocumentNavigation),
    SessionHistoryUpdate {
        source_document: RendererDocumentLifecycleIdentity,
        update: moli_page_types::SessionHistoryUpdate,
    },
    TopLevelLocationNavigation(RendererDocumentSourcedTopLevelLocationNavigation),
    StopTopLevelLocationNavigation {
        source_document: RendererDocumentLifecycleIdentity,
    },
    TopLevelHistoryTraversal(RendererPendingTopLevelHistoryTraversal),
    SubresourceFetchPause {
        network_pause: Option<crate::network::ResourceInterceptionPause>,
        source_document: RendererDocumentLifecycleIdentity,
        info: Box<PendingSubresourceFetchInfo>,
        /// Set for a child frame navigation, whose Network request id is the
        /// loader id of the Document it creates.
        child_document: Option<Box<crate::protocol_types::ChildDocumentFetchRequest>>,
        redirect: Option<Box<crate::protocol_types::NavigationRedirect>>,
    },
    SubresourceContinue {
        response_body: Option<crate::network::ResourceInterceptionBody>,
        network_pause: Option<crate::network::ResourceInterceptionPause>,
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
    #[cfg(feature = "devtools")]
    WebMcp(moli_page_types::RendererWebMcpObservation),
    MainDocumentCommit {
        source_document: RendererDocumentLifecycleIdentity,
        commit: RendererMainDocumentCommit,
    },
    DocumentTitleChanged(RendererDocumentTitleChanged),
    DocumentLifecycle(RendererDocumentLifecycleEvent),
    Network {
        source_document: RendererDocumentLifecycleIdentity,
        item: ScriptNetworkOutputItem,
    },
    RuntimeBinding(PendingRuntimeBindingCall),
    #[cfg(feature = "devtools")]
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
    #[cfg(feature = "devtools")]
    NativeTerminal(super::RendererNativeCommandTerminal),
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
            #[cfg(feature = "devtools")]
            RendererOutputItem::NativeTerminal(_) => None,
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

#[cfg(feature = "molio")]
impl super::RendererOutputPublication {
    pub(super) fn prepare_desktop_downloads(&mut self, limit: usize) {
        use std::cmp::Reverse;

        use crate::runtime::{RendererDownloadBody, RendererPendingDownloadActivation};

        let mut has_downloads = false;
        for record in &mut self.records {
            let RendererOutputItem::OwnerAction(RendererOwnerAction::Download(download)) =
                &mut record.item
            else {
                continue;
            };
            has_downloads = true;
            if let Some(response) = &mut download.response
                && let RendererDownloadBody::Buffered(bytes) = &mut response.body
            {
                response.body = RendererDownloadBody::Blob(std::mem::take(bytes).into());
            }
        }
        if !has_downloads {
            return;
        }

        let mut excess = self.transport_charge_bytes().saturating_sub(limit);
        if excess == 0 {
            return;
        }
        let mut downloads: Vec<_> = self
            .records
            .iter_mut()
            .filter(|record| {
                matches!(
                    record.item,
                    RendererOutputItem::OwnerAction(RendererOwnerAction::Download(_))
                )
            })
            .map(|record| (record.transport_charge_bytes(), record))
            .collect();
        downloads.sort_unstable_by_key(|(charge, _)| Reverse(*charge));
        for (previous_charge, record) in downloads {
            if excess == 0 {
                break;
            }
            if previous_charge <= super::transport_memory::RECORD_FLOOR_BYTES {
                break;
            }
            let RendererOutputItem::OwnerAction(RendererOwnerAction::Download(download)) =
                &mut record.item
            else {
                unreachable!("only desktop download records are selected");
            };
            let name = download
                .suggested_filename
                .as_deref()
                .map(|name| name[..name.floor_char_boundary(255)].to_owned());
            if let Some(response) = &download.response {
                response.body.cancel();
            }
            *download =
                RendererPendingDownloadActivation::new("about:blank".to_owned(), None, name, None);
            download.desktop_failure = Some("Download metadata exceeds the output limit");
            excess = excess
                .saturating_sub(previous_charge.saturating_sub(record.transport_charge_bytes()));
        }
    }
}

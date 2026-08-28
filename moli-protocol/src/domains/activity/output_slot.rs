use std::{future::Future, pin::Pin};

use crate::conn::CdpConnection;

use super::contextual_projection::ProtocolOutputProjectionContext;
use super::output_payloads::{ProtocolOutputPayload, ProtocolOutputPayloads};

/// Whether consuming prepared output is required for browser-owner progress
/// or only projects an already-settled fact to a protocol subscriber.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::domains) enum ProtocolOutputDelivery {
    OwnerAction,
    ProtocolObservation,
}

/// Where concrete output produced while a Runtime command is pending becomes
/// visible relative to that exact command's response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::domains) enum ProtocolOutputResponseOrder {
    BeforeResponse,
    AfterResponse,
}

/// Closed set of protocol output families.
///
/// This enum is the projection identity. It replaces string-keyed projection
/// handles and the canonical global slot registry, so adding a family forces
/// exhaustive delivery, response-order, and projection decisions at compile
/// time.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::domains) enum ProtocolOutputSlot {
    PendingSubresourceContinueEvents,
    TopLevelFocus,
    TopLevelClose,
    TopLevelLocationNavigation,
    TopLevelHistoryTraversal,
    FileChooser,
    Download,
    JavascriptDialog,
    WindowOpen,
    Popup,
    SharedWorkerTargetLifecycle,
    ServiceWorkerTargetLifecycle,
    DedicatedWorkerTargetLifecycle,
    Audits,
    Console,
    Log,
    RendererNetworkLive,
    NetworkBacklog,
    SubresourceFetchInterception,
    RuntimeBindingCalls,
    DomMutations,
    RuntimeInspectorMessages,
    RuntimeInspectorPostResponseMessages,
    MainDocumentCommit,
    DocumentTitleChanged,
    DocumentLifecycle,
    RuntimeObservable,
    DomStorage,
    ChildFrameActivity,
    SameDocumentNavigation,
}

impl ProtocolOutputSlot {
    pub(in crate::domains::activity) const fn delivery(self) -> ProtocolOutputDelivery {
        match self {
            Self::PendingSubresourceContinueEvents
            | Self::TopLevelFocus
            | Self::TopLevelClose
            | Self::FileChooser
            | Self::Download
            | Self::JavascriptDialog
            | Self::Popup
            | Self::SharedWorkerTargetLifecycle
            | Self::ServiceWorkerTargetLifecycle
            | Self::DedicatedWorkerTargetLifecycle
            | Self::RuntimeInspectorMessages
            | Self::RuntimeInspectorPostResponseMessages
            | Self::RuntimeObservable
            | Self::ChildFrameActivity
            | Self::SameDocumentNavigation
            | Self::TopLevelLocationNavigation
            | Self::TopLevelHistoryTraversal => ProtocolOutputDelivery::OwnerAction,
            Self::WindowOpen
            | Self::Audits
            | Self::Console
            | Self::Log
            | Self::RendererNetworkLive
            | Self::NetworkBacklog
            | Self::SubresourceFetchInterception
            | Self::RuntimeBindingCalls
            | Self::DomMutations
            | Self::MainDocumentCommit
            | Self::DocumentTitleChanged
            | Self::DocumentLifecycle
            | Self::DomStorage => ProtocolOutputDelivery::ProtocolObservation,
        }
    }

    pub(in crate::domains::activity) const fn command_response_order(
        self,
    ) -> ProtocolOutputResponseOrder {
        match self {
            Self::TopLevelClose
            | Self::TopLevelLocationNavigation
            | Self::TopLevelHistoryTraversal
            | Self::Download
            | Self::SharedWorkerTargetLifecycle
            | Self::ServiceWorkerTargetLifecycle
            | Self::ChildFrameActivity
            | Self::RuntimeInspectorPostResponseMessages => {
                ProtocolOutputResponseOrder::AfterResponse
            }
            Self::PendingSubresourceContinueEvents
            // Blink's file-input activation probe queues
            // Page.fileChooserOpened synchronously. A script may continue into
            // document.open(), but Chromium still flushes the chooser event
            // before the invoking Runtime.evaluate response.
            | Self::FileChooser
            | Self::TopLevelFocus
            | Self::JavascriptDialog
            | Self::WindowOpen
            | Self::Popup
            // A blob: DedicatedWorker can be created by a Runtime command that
            // awaits the worker's first message. With pause-on-start auto-attach,
            // Chromium exposes attachedToTarget before that command completes so
            // clients can send Runtime.runIfWaitingForDebugger and unblock it.
            | Self::DedicatedWorkerTargetLifecycle
            | Self::Audits
            | Self::Console
            | Self::Log
            | Self::RendererNetworkLive
            | Self::NetworkBacklog
            | Self::SubresourceFetchInterception
            | Self::RuntimeBindingCalls
            | Self::DomMutations
            | Self::RuntimeInspectorMessages
            | Self::MainDocumentCommit
            | Self::DocumentTitleChanged
            | Self::DocumentLifecycle
            | Self::RuntimeObservable
            | Self::DomStorage
            | Self::SameDocumentNavigation => ProtocolOutputResponseOrder::BeforeResponse,
        }
    }

    pub(in crate::domains::activity) fn project_async<'a>(
        self,
        conn: &'a mut CdpConnection,
        context: &'a mut ProtocolOutputProjectionContext<'_>,
        payloads: Option<&'a mut ProtocolOutputPayloads>,
    ) -> Pin<Box<dyn Future<Output = ()> + 'a>> {
        Box::pin(async move {
            match self {
                Self::PendingSubresourceContinueEvents => {
                    crate::domains::network::project_pending_subresource_continue_async(
                        conn, context, payloads,
                    )
                    .await;
                }
                Self::NetworkBacklog => {
                    crate::domains::network::project_network_backlog_async(conn, context, payloads)
                        .await;
                }
                Self::RendererNetworkLive => {
                    crate::domains::network::project_renderer_network_live_async(
                        conn, context, payloads,
                    )
                    .await;
                }
                Self::SubresourceFetchInterception => {
                    crate::domains::network::project_subresource_fetch_interception_async(
                        conn, context, payloads,
                    )
                    .await;
                }
                Self::RuntimeBindingCalls => {
                    crate::domains::runtime::project_runtime_binding_calls_async(
                        conn, context, payloads,
                    )
                    .await;
                }
                Self::RuntimeInspectorMessages => {
                    crate::domains::runtime::project_runtime_inspector_messages_async(
                        conn, context, payloads,
                    )
                    .await;
                }
                Self::RuntimeInspectorPostResponseMessages => {
                    crate::domains::runtime::project_runtime_inspector_post_response_messages_async(
                        conn, context, payloads,
                    )
                    .await;
                }
                Self::Audits => {
                    crate::domains::observable_output::project_audits_async(
                        conn, context, payloads,
                    )
                    .await;
                }
                Self::Console => {
                    crate::domains::observable_output::project_console_async(
                        conn, context, payloads,
                    )
                    .await;
                }
                Self::Log => {
                    crate::domains::observable_output::project_log_async(conn, context, payloads)
                        .await;
                }
                Self::RuntimeObservable => {
                    crate::domains::observable_output::project_runtime_observable_async(
                        conn, context, payloads,
                    )
                    .await;
                }
                Self::DomMutations => {
                    crate::domains::dom::project_dom_mutations_async(conn, context, payloads).await;
                }
                Self::DomStorage => {
                    crate::domains::dom_storage::project_dom_storage_async(context, payloads).await;
                }
                Self::MainDocumentCommit => {
                    crate::domains::page::project_main_document_commit_async(
                        conn, context, payloads,
                    )
                    .await;
                }
                Self::Download
                | Self::TopLevelFocus
                | Self::TopLevelClose
                | Self::FileChooser
                | Self::JavascriptDialog
                | Self::WindowOpen
                | Self::Popup
                | Self::DocumentTitleChanged
                | Self::DocumentLifecycle
                | Self::ChildFrameActivity
                | Self::SameDocumentNavigation
                | Self::TopLevelLocationNavigation
                | Self::TopLevelHistoryTraversal => {
                    crate::domains::page::project_page_output_async(self, conn, context, payloads)
                        .await;
                }
                Self::SharedWorkerTargetLifecycle
                | Self::ServiceWorkerTargetLifecycle
                | Self::DedicatedWorkerTargetLifecycle => {
                    crate::domains::target::project_worker_target_output_async(
                        self, conn, context, payloads,
                    )
                    .await;
                }
            }
        })
    }
}

/// Assembly capability for a domain to append one closed projection family
/// and its move-only, compiler-known payload aggregate.
pub(in crate::domains) trait ProtocolOutputSink {
    fn push_produced_slot(&mut self, slot: ProtocolOutputSlot);

    fn push_prepared_payload(&mut self, payload: ProtocolOutputPayload);
}

#[cfg(test)]
mod tests {
    use super::{ProtocolOutputDelivery, ProtocolOutputResponseOrder, ProtocolOutputSlot};

    #[test]
    fn closed_output_families_have_explicit_ordering_metadata() {
        use ProtocolOutputDelivery::{OwnerAction, ProtocolObservation};
        use ProtocolOutputResponseOrder::{AfterResponse, BeforeResponse};
        use ProtocolOutputSlot::*;

        let cases = [
            (
                PendingSubresourceContinueEvents,
                OwnerAction,
                BeforeResponse,
            ),
            (TopLevelClose, OwnerAction, AfterResponse),
            (TopLevelFocus, OwnerAction, BeforeResponse),
            (TopLevelLocationNavigation, OwnerAction, AfterResponse),
            (TopLevelHistoryTraversal, OwnerAction, AfterResponse),
            (FileChooser, OwnerAction, BeforeResponse),
            (Download, OwnerAction, AfterResponse),
            (JavascriptDialog, OwnerAction, BeforeResponse),
            (WindowOpen, ProtocolObservation, BeforeResponse),
            (Popup, OwnerAction, BeforeResponse),
            (SharedWorkerTargetLifecycle, OwnerAction, AfterResponse),
            (ServiceWorkerTargetLifecycle, OwnerAction, AfterResponse),
            (DedicatedWorkerTargetLifecycle, OwnerAction, BeforeResponse),
            (Audits, ProtocolObservation, BeforeResponse),
            (Console, ProtocolObservation, BeforeResponse),
            (Log, ProtocolObservation, BeforeResponse),
            (NetworkBacklog, ProtocolObservation, BeforeResponse),
            (
                SubresourceFetchInterception,
                ProtocolObservation,
                BeforeResponse,
            ),
            (RuntimeBindingCalls, ProtocolObservation, BeforeResponse),
            (DomMutations, ProtocolObservation, BeforeResponse),
            (RuntimeInspectorMessages, OwnerAction, BeforeResponse),
            (
                RuntimeInspectorPostResponseMessages,
                OwnerAction,
                AfterResponse,
            ),
            (MainDocumentCommit, ProtocolObservation, BeforeResponse),
            (DocumentTitleChanged, ProtocolObservation, BeforeResponse),
            (DocumentLifecycle, ProtocolObservation, BeforeResponse),
            (RuntimeObservable, OwnerAction, BeforeResponse),
            (DomStorage, ProtocolObservation, BeforeResponse),
            (ChildFrameActivity, OwnerAction, AfterResponse),
            (SameDocumentNavigation, OwnerAction, BeforeResponse),
        ];

        let unique = cases
            .iter()
            .map(|(output, _, _)| *output)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), cases.len());
        for (output, delivery, response_order) in cases {
            assert_eq!(output.delivery(), delivery, "{output:?}");
            assert_eq!(
                output.command_response_order(),
                response_order,
                "{output:?}"
            );
        }
    }
}

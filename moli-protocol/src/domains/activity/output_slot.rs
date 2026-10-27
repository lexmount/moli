use std::{future::Future, pin::Pin};

use anyhow::Result;

use crate::conn::{CdpConnection, CommandWorkflowContext, ProtocolWorkflowDriver};

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
    WebMcp,
    PendingSubresourceContinueEvents,
    TopLevelLocationNavigation,
    TopLevelHistoryTraversal,
    FileChooser,
    Download,
    JavascriptDialog,
    WindowOpen,
    Popup,
    CloseAuxiliaryWindow,
    NavigateAuxiliaryWindow,
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
    SessionHistoryUpdate,
}

impl ProtocolOutputSlot {
    pub(in crate::domains::activity) const fn delivery(self) -> ProtocolOutputDelivery {
        match self {
            Self::PendingSubresourceContinueEvents
            | Self::FileChooser
            | Self::Download
            | Self::JavascriptDialog
            | Self::Popup
            | Self::CloseAuxiliaryWindow
            | Self::NavigateAuxiliaryWindow
            | Self::SharedWorkerTargetLifecycle
            | Self::ServiceWorkerTargetLifecycle
            | Self::DedicatedWorkerTargetLifecycle
            | Self::RuntimeInspectorMessages
            | Self::RuntimeInspectorPostResponseMessages
            | Self::RuntimeObservable
            | Self::ChildFrameActivity
            | Self::SessionHistoryUpdate
            | Self::SameDocumentNavigation
            | Self::TopLevelLocationNavigation
            | Self::TopLevelHistoryTraversal => ProtocolOutputDelivery::OwnerAction,
            Self::WindowOpen
            | Self::WebMcp
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
            Self::CloseAuxiliaryWindow
            | Self::NavigateAuxiliaryWindow
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
            | Self::WebMcp
            | Self::SessionHistoryUpdate
            | Self::SameDocumentNavigation => ProtocolOutputResponseOrder::BeforeResponse,
        }
    }

    /// Consumes only already-frozen facts and finite owner-state changes.
    /// Native waits are returned to the ordinary async projection workflow.
    pub(in crate::domains::activity) fn project_finite(
        self,
        conn: &mut CdpConnection,
        context: &mut ProtocolOutputProjectionContext<'_>,
        payloads: &mut ProtocolOutputPayloads,
    ) -> Result<(), Self> {
        match self {
            Self::WebMcp => crate::domains::web_mcp::project_web_mcp(context, payloads),
            Self::NetworkBacklog => {
                crate::domains::network::project_network_backlog(conn, context, payloads)
            }
            Self::RendererNetworkLive => {
                crate::domains::network::project_renderer_network_live(conn, context, payloads)
            }
            Self::SubresourceFetchInterception => {
                crate::domains::network::project_subresource_fetch_interception(
                    conn, context, payloads,
                )
            }
            Self::RuntimeBindingCalls => {
                crate::domains::runtime::project_runtime_binding_calls(conn, context, payloads)
            }
            Self::Audits => {
                crate::domains::observable_output::project_audits(conn, context, payloads)
            }
            Self::Console => {
                crate::domains::observable_output::project_console(conn, context, payloads)
            }
            Self::Log => crate::domains::observable_output::project_log(conn, context, payloads),
            Self::RuntimeObservable => {
                crate::domains::observable_output::project_runtime_observable(
                    conn, context, payloads,
                )
            }
            Self::DomMutations => {
                crate::domains::dom::project_dom_mutations(conn, context, payloads)
            }
            Self::DomStorage => crate::domains::dom_storage::project_dom_storage(context, payloads),
            Self::MainDocumentCommit => {
                crate::domains::page::project_main_document_commit(conn, context, payloads)
            }
            Self::DocumentLifecycle
            | Self::DocumentTitleChanged
            | Self::JavascriptDialog
            | Self::WindowOpen
            | Self::ChildFrameActivity
            | Self::NavigateAuxiliaryWindow
            | Self::SessionHistoryUpdate
            | Self::TopLevelLocationNavigation => {
                return crate::domains::page::project_page_output_finite(
                    self, conn, context, payloads,
                );
            }
            _ => return Err(self),
        }
        Ok(())
    }

    pub(super) fn prepare_wait(
        self,
        conn: &mut CdpConnection,
        owner: &crate::conn::CommandOwnerScope,
        payloads: &mut ProtocolOutputPayloads,
    ) -> Option<PreparedProtocolOutputWait> {
        match self {
            Self::RuntimeInspectorMessages | Self::RuntimeInspectorPostResponseMessages => {
                crate::domains::runtime::PreparedRuntimeInspectorProjection::take(self, payloads)
                    .map(PreparedProtocolOutputWait::Runtime)
            }
            Self::Download | Self::FileChooser => {
                crate::domains::input::PreparedInputOutputProjection::take(
                    self, conn, owner, payloads,
                )
                .map(PreparedProtocolOutputWait::Input)
            }
            Self::SharedWorkerTargetLifecycle
            | Self::ServiceWorkerTargetLifecycle
            | Self::DedicatedWorkerTargetLifecycle => {
                crate::domains::target::WorkerTargetOutputProjection::take(payloads)
                    .map(PreparedProtocolOutputWait::Worker)
            }
            _ => unreachable!("finite projection selects the native output family"),
        }
    }

    pub(in crate::domains) fn project_async<'a>(
        self,
        conn: &'a mut CdpConnection,
        context: &'a mut ProtocolOutputProjectionContext<'_>,
        payloads: &'a mut ProtocolOutputPayloads,
    ) -> Pin<Box<dyn Future<Output = ()> + 'a>> {
        Box::pin(async move {
            if self.project_finite(conn, context, payloads).is_ok() {
                return;
            }
            match self {
                Self::PendingSubresourceContinueEvents => {
                    crate::domains::network::project_pending_subresource_continue_async(
                        conn, context, payloads,
                    )
                    .await;
                }
                Self::RuntimeInspectorMessages
                | Self::RuntimeInspectorPostResponseMessages
                | Self::Download
                | Self::FileChooser
                | Self::SharedWorkerTargetLifecycle
                | Self::ServiceWorkerTargetLifecycle
                | Self::DedicatedWorkerTargetLifecycle => {
                    if let Some(wait) = self.prepare_wait(conn, context.owner(), payloads) {
                        let mut local = crate::conn::CommandDispatchContext::new(
                            context.command.response_flush().clone(),
                        );
                        let (mut output, _scope) =
                            CommandWorkflowContext::take_from(&mut local, None, None);
                        let result = wait
                            .run(&mut ProtocolWorkflowDriver::inline(conn), &mut output)
                            .await;
                        output.finish_projection_inline(conn, context.command);
                        result.expect("exclusive output projection retains its connection owner");
                    }
                }
                Self::Popup
                | Self::CloseAuxiliaryWindow
                | Self::SameDocumentNavigation
                | Self::TopLevelHistoryTraversal => {
                    crate::domains::page::project_page_output_async(self, conn, context, payloads)
                        .await;
                }
                Self::WebMcp
                | Self::NetworkBacklog
                | Self::RendererNetworkLive
                | Self::SubresourceFetchInterception
                | Self::RuntimeBindingCalls
                | Self::Audits
                | Self::Console
                | Self::Log
                | Self::RuntimeObservable
                | Self::DomMutations
                | Self::DomStorage
                | Self::MainDocumentCommit
                | Self::DocumentLifecycle
                | Self::DocumentTitleChanged
                | Self::JavascriptDialog
                | Self::WindowOpen
                | Self::ChildFrameActivity
                | Self::NavigateAuxiliaryWindow
                | Self::SessionHistoryUpdate
                | Self::TopLevelLocationNavigation => {
                    unreachable!("finite output slots have already been consumed")
                }
            }
        })
    }
}

pub(crate) enum PreparedProtocolOutputWait {
    Worker(crate::domains::target::WorkerTargetOutputProjection),
    Runtime(crate::domains::runtime::PreparedRuntimeInspectorProjection),
    Input(crate::domains::input::PreparedInputOutputProjection),
}

impl PreparedProtocolOutputWait {
    pub(in crate::domains::activity) async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver<'_>,
        context: &mut CommandWorkflowContext,
    ) -> Result<()> {
        match self {
            Self::Worker(projection) => projection.run(driver, context).await,
            Self::Runtime(projection) => projection.run(driver, context).await,
            Self::Input(projection) => {
                projection.run(driver, context).await;
                Ok(())
            }
        }
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
            (WebMcp, ProtocolObservation, BeforeResponse),
            (ChildFrameActivity, OwnerAction, AfterResponse),
            (SameDocumentNavigation, OwnerAction, BeforeResponse),
            (SessionHistoryUpdate, OwnerAction, BeforeResponse),
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

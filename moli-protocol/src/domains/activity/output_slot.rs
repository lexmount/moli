use anyhow::Result;
use futures_util::{FutureExt as _, future::LocalBoxFuture};

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
            | Self::DocumentLifecycle => ProtocolOutputDelivery::ProtocolObservation,
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
            | Self::TopLevelLocationNavigation
            | Self::TopLevelHistoryTraversal
            | Self::SameDocumentNavigation => {
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
        context: &mut CommandWorkflowContext,
    ) -> Option<PreparedProtocolOutputWait> {
        match self {
            Self::CloseAuxiliaryWindow => {
                crate::domains::page::PreparedPageOutputClose::take(payloads)
                    .map(PreparedProtocolOutputWait::Close)
            }
            Self::Popup => crate::domains::page::PreparedPopupOutputProjection::take(payloads)
                .map(PreparedProtocolOutputWait::Popup),
            Self::PendingSubresourceContinueEvents => {
                crate::domains::network::PreparedSubresourceProjection::take(
                    conn, owner, payloads, context,
                )
                .map(PreparedProtocolOutputWait::Subresource)
            }
            Self::RuntimeInspectorMessages | Self::RuntimeInspectorPostResponseMessages => {
                crate::domains::runtime::PreparedRuntimeInspectorProjection::take(
                    conn, context, self, payloads,
                )
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
    #[cfg(test)]
    pub(in crate::domains) async fn project_async(
        self,
        conn: &mut CdpConnection,
        context: &mut ProtocolOutputProjectionContext<'_>,
        payloads: &mut ProtocolOutputPayloads,
    ) {
        if self.project_finite(conn, context, payloads).is_ok() {
            return;
        }
        {
            let mut local =
                crate::conn::CommandDispatchContext::new(context.command.response_flush().clone());
            let (mut output, _scope) = CommandWorkflowContext::take_from(&mut local, None, None);
            if let Some(pending) = self.prepare_wait(conn, context.owner(), payloads, &mut output) {
                pending
                    .run(&mut ProtocolWorkflowDriver::inline(conn), &mut output)
                    .await
                    .expect("inline test projection retains its owner");
            }
            output.finish_projection_inline(conn, context.command);
        }
    }
}

pub(crate) enum PreparedProtocolOutputWait {
    Close(crate::domains::page::PreparedPageOutputClose),
    Popup(crate::domains::page::PreparedPopupOutputProjection),
    Subresource(crate::domains::network::PreparedSubresourceProjection),
    Worker(crate::domains::target::WorkerTargetOutputProjection),
    Runtime(crate::domains::runtime::PreparedRuntimeInspectorProjection),
    Input(crate::domains::input::PreparedInputOutputProjection),
    RuntimeOutputs(Vec<super::ReadyRuntimeCommandOutputProjection>),
}

impl PreparedProtocolOutputWait {
    pub(in crate::domains::activity) fn run<'a>(
        self,
        driver: &'a mut ProtocolWorkflowDriver<'_>,
        context: &'a mut CommandWorkflowContext,
    ) -> LocalBoxFuture<'a, Result<()>> {
        match self {
            Self::Close(projection) => projection.run(driver, context).boxed_local(),
            Self::Popup(projection) => projection.run(driver, context).boxed_local(),
            Self::Subresource(projection) => projection.run(driver, context).boxed_local(),
            Self::Worker(projection) => projection.run(driver, context).boxed_local(),
            Self::Runtime(projection) => projection.run(driver, context).boxed_local(),
            Self::RuntimeOutputs(ready) => {
                super::ReadyRuntimeCommandOutputProjection::run_ready(ready, driver, context)
                    .boxed_local()
            }
            Self::Input(projection) => async move {
                projection.run(driver, context).await;
                Ok(())
            }
            .boxed_local(),
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

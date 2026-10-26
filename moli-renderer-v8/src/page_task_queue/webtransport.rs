//! WebTransport tasks share HTML's networking FIFO; V8 payloads stay in their Host.
use super::{
    PageWindowDocumentTaskTargetEffect, PageWindowDocumentTaskTurnAction,
    RendererPageNetworkingRoute, RendererPageNetworkingTask, RendererPageWindowDocumentTask,
    RendererPageWindowDocumentTaskOwner,
};
use crate::{native_bridge::WindowExecutionContextIdentity, runtime::RendererDocumentToken};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageWebTransportTaskId(u64);
impl RendererPageWebTransportTaskId {
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageWebTransportTaskKind {
    Complete,
}
pub(crate) type RendererPageWebTransportOwner =
    RendererPageWindowDocumentTaskOwner<WindowExecutionContextIdentity>;
pub(crate) type RendererPageWebTransportTask = RendererPageWindowDocumentTask<
    RendererPageWebTransportTaskId,
    RendererPageWebTransportTaskKind,
    WindowExecutionContextIdentity,
>;
pub(crate) type PageWebTransportTurnAction = PageWindowDocumentTaskTurnAction<
    RendererPageWebTransportTaskId,
    RendererPageWebTransportTaskKind,
    WindowExecutionContextIdentity,
>;
pub(crate) type PageWebTransportTargetEffect =
    PageWindowDocumentTaskTargetEffect<WindowExecutionContextIdentity>;
pub(crate) type PageWebTransportTurnOutcome =
    crate::runtime::PageOwnerTurnOutcome<PageWebTransportTurnAction>;
#[derive(Clone, Debug)]
pub(crate) struct RendererPageWebTransportSender {
    networking: RendererPageNetworkingRoute,
    root_document: RendererDocumentToken,
}
impl RendererPageWebTransportSender {
    pub(super) fn new(
        networking: RendererPageNetworkingRoute,
        root_document: RendererDocumentToken,
    ) -> Self {
        Self {
            networking,
            root_document,
        }
    }
    pub(crate) fn send(
        &self,
        target: WindowExecutionContextIdentity,
        id: RendererPageWebTransportTaskId,
        kind: RendererPageWebTransportTaskKind,
    ) -> bool {
        self.networking
            .send(RendererPageNetworkingTask::WebTransport(
                RendererPageWebTransportTask::new(
                    RendererPageWebTransportOwner::new(self.root_document, target),
                    id,
                    kind,
                ),
            ))
            .is_ok()
    }
}

//! WebRTC tasks share HTML's networking FIFO; V8 payloads stay in their Host.
use super::{
    PageWindowDocumentTaskTargetEffect, PageWindowDocumentTaskTurnAction,
    RendererPageNetworkingRoute, RendererPageNetworkingTask, RendererPageWindowDocumentTask,
    RendererPageWindowDocumentTaskOwner,
};
use crate::{native_bridge::WindowDocumentTaskTarget, runtime::RendererDocumentToken};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageWebRtcTaskId(u64);
impl RendererPageWebRtcTaskId {
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageWebRtcTaskKind {
    TrackEnded,
    NegotiationNeeded,
    CreateOffer,
    SetLocalDescription,
    ReplaceTrack,
    CompleteReplaceTrack,
    ClearRtpParameters,
    SetRtpParameters,
    GetStats,
    StartDataChannelClose,
    DataChannelClosed,
}
pub(crate) type RendererPageWebRtcOwner = RendererPageWindowDocumentTaskOwner;
pub(crate) type RendererPageWebRtcTask =
    RendererPageWindowDocumentTask<RendererPageWebRtcTaskId, RendererPageWebRtcTaskKind>;
pub(crate) type PageWebRtcTurnAction =
    PageWindowDocumentTaskTurnAction<RendererPageWebRtcTaskId, RendererPageWebRtcTaskKind>;
pub(crate) type PageWebRtcTargetEffect = PageWindowDocumentTaskTargetEffect;
pub(crate) type PageWebRtcTurnOutcome = crate::runtime::PageOwnerTurnOutcome<PageWebRtcTurnAction>;
#[derive(Clone, Debug)]
pub(crate) struct RendererPageWebRtcSender {
    networking: RendererPageNetworkingRoute,
    root_document: RendererDocumentToken,
}
impl RendererPageWebRtcSender {
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
        target: WindowDocumentTaskTarget,
        id: RendererPageWebRtcTaskId,
        kind: RendererPageWebRtcTaskKind,
    ) -> bool {
        self.networking
            .send(RendererPageNetworkingTask::WebRtc(
                RendererPageWebRtcTask::new(
                    RendererPageWebRtcOwner::new(self.root_document, target),
                    id,
                    kind,
                ),
            ))
            .is_ok()
    }
}

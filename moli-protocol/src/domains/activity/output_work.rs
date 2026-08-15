#[cfg(feature = "test-support")]
use crate::conn::TargetPageResidenceIdentity;
use crate::conn::{BackgroundProtocolEvent, CdpConnection, TargetPageProtocolAttachmentIdentity};

/// Result of consuming one armed root-frame stopped-loading observation.
///
/// The observation is an exact-Document fact and is consumed in both cases.
/// `Unobserved` means that no attachment had enabled the CDP `Page` domain at
/// settlement time, so there is deliberately no protocol output to enqueue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RootFrameStoppedLoadingSettlement {
    Published,
    Unobserved,
}

/// Broken owner invariants detected while settling an armed observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RootFrameStoppedLoadingSettlementError {
    /// The caller reported that the exact observation was armed, but the Page
    /// slot no longer owned its pending stopped-loading fact.
    MissingArmedObservation,
    /// At least one `Page` subscriber existed, but its exact Page attachment
    /// could not be frozen for deferred delivery.
    SubscribedAttachmentUnavailable,
}

/// Durable protocol output whose payload and delivery routes are frozen.
///
/// This move-only capability does not name a source that should be scanned
/// later. It owns the concrete fact that was published and the exact Page
/// attachments frozen with it. Materialization may only authorize those
/// frozen routes; it must not rediscover payload or destinations from the
/// then-current Page, Document, target, or session.
#[derive(Debug, Eq, PartialEq)]
pub struct ProtocolOutputWork {
    payload: ProtocolOutputPayload,
}

#[derive(Debug, Eq, PartialEq)]
enum ProtocolOutputPayload {
    RootFrameStoppedLoading(RootFrameStoppedLoadingOutput),
}

/// Concrete payload and frozen routes for root-frame stopped-loading.
#[derive(Debug, Eq, PartialEq)]
struct RootFrameStoppedLoadingOutput {
    attachments: Vec<TargetPageProtocolAttachmentIdentity>,
    frame_id: String,
    loader_id: String,
}

impl ProtocolOutputWork {
    pub(crate) fn root_frame_stopped_loading(
        attachments: Vec<TargetPageProtocolAttachmentIdentity>,
        frame_id: String,
        loader_id: String,
    ) -> Self {
        assert!(
            !attachments.is_empty(),
            "protocol output work must own at least one exact delivery route"
        );
        Self {
            payload: ProtocolOutputPayload::RootFrameStoppedLoading(
                RootFrameStoppedLoadingOutput {
                    attachments,
                    frame_id,
                    loader_id,
                },
            ),
        }
    }

    #[cfg(feature = "test-support")]
    pub(super) fn root_frame_stopped_loading_for_test_support(
        session_ids: Vec<Option<String>>,
        frame_id: String,
        loader_id: String,
    ) -> Self {
        Self::root_frame_stopped_loading_for_target_test_support(
            session_ids,
            "__protocol_output_test_context__".to_owned(),
            "__protocol_output_test_target__".to_owned(),
            frame_id,
            loader_id,
        )
    }

    #[cfg(feature = "test-support")]
    pub(super) fn root_frame_stopped_loading_for_target_test_support(
        session_ids: Vec<Option<String>>,
        browser_context_id: String,
        target_id: String,
        frame_id: String,
        loader_id: String,
    ) -> Self {
        let page_owner = TargetPageResidenceIdentity::new(
            browser_context_id,
            Some(target_id),
            crate::conn::TargetPageAttachmentId::allocate(),
        );
        let attachments = session_ids
            .into_iter()
            .map(|session_id| {
                TargetPageProtocolAttachmentIdentity::new(page_owner.clone(), session_id)
            })
            .collect();
        Self::root_frame_stopped_loading(attachments, frame_id, loader_id)
    }

    pub fn is_root_frame_stopped_loading(&self) -> bool {
        matches!(
            &self.payload,
            ProtocolOutputPayload::RootFrameStoppedLoading(_)
        )
    }

    pub(crate) fn navigation_gate_target_id(&self) -> Option<&str> {
        match &self.payload {
            ProtocolOutputPayload::RootFrameStoppedLoading(output) => {
                let target_id = output.attachments.first()?.page_owner().target_id()?;
                debug_assert!(
                    output.attachments.iter().all(|attachment| {
                        attachment.page_owner().target_id() == Some(target_id)
                    })
                );
                Some(target_id)
            }
        }
    }

    pub(crate) fn into_background_events(
        self,
        conn: &CdpConnection,
    ) -> Vec<BackgroundProtocolEvent> {
        match self.payload {
            ProtocolOutputPayload::RootFrameStoppedLoading(output) => {
                let mut events = Vec::new();
                for attachment in output.attachments {
                    if !conn.target_page_protocol_attachment_identity_is_current(&attachment) {
                        continue;
                    }
                    crate::domains::page::emit_navigation_frame_stopped_loading_background_events(
                        &mut events,
                        attachment.session_id(),
                        &output.frame_id,
                        &output.loader_id,
                    );
                }
                events
            }
        }
    }
}

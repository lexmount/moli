mod fence;
mod identity;
mod item;
mod transport;
mod transport_memory;
mod turn_journal;

pub use fence::RendererOutputFence;
pub use identity::{
    RendererOutputCursor, RendererOutputFenceLeaseId, RendererOutputResidenceIdentity,
    RendererOutputStreamCloseReason, RendererOutputStreamControl, RendererOutputStreamEpoch,
    RendererOutputStreamIdentity, RendererPageOutputOwnerReservationId,
};
pub(crate) use item::PendingRendererOutputRecord;
pub use item::{
    RendererDocumentTitleChanged, RendererOutputItem, RendererOutputRecord, RendererOwnerAction,
    RendererProtocolObservation, RendererTopLevelCloseSource,
};
pub use transport::{
    RendererOutputTransportDiagnostics, RendererOutputTransportMessage,
    RendererOutputTransportReceiver, RendererOutputTransportSendError,
    RendererOutputTransportSender, renderer_output_transport_channel,
};
#[cfg(test)]
pub(in crate::runtime) use transport::{
    RendererOutputTransportTestLimits, renderer_output_transport_channel_with_test_limits,
};
pub(crate) use turn_journal::RendererTurnOutputJournal;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RendererOutputPublicationAdmission {
    Observation,
    EssentialOwnerProgress,
    EssentialTerminalResponse,
}

/// Move-owned output frozen at one renderer owner-turn settlement boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct RendererOutputPublication {
    cursor: RendererOutputCursor,
    ordering: RendererOutputPublicationOrdering,
    admission: RendererOutputPublicationAdmission,
    records: Vec<RendererOutputRecord>,
}

/// Ordering information that cannot be reconstructed from a concrete record
/// after its renderer turn has ended.
///
/// This grants no capture capability and does not name an HTML task source.
/// It only preserves the observable rule that post-load Page effects cannot
/// overtake an exact pending `Page.loadEventFired` observation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RendererOutputPublicationOrdering {
    #[default]
    Unconstrained,
    AfterPendingPageLoad {
        source_document: crate::runtime::RendererDocumentLifecycleIdentity,
    },
}

impl RendererOutputPublication {
    pub(crate) fn new(cursor: RendererOutputCursor, records: Vec<RendererOutputRecord>) -> Self {
        assert!(
            !records.is_empty(),
            "renderer output publications must be non-empty"
        );
        let admission = if records.iter().any(RendererOutputRecord::is_owner_action) {
            RendererOutputPublicationAdmission::EssentialOwnerProgress
        } else {
            RendererOutputPublicationAdmission::Observation
        };
        Self {
            cursor,
            ordering: RendererOutputPublicationOrdering::Unconstrained,
            admission,
            records,
        }
    }

    pub(crate) fn with_ordering(mut self, ordering: RendererOutputPublicationOrdering) -> Self {
        self.ordering = ordering;
        self
    }

    /// Protects the one terminal response owned by a DevTools session from
    /// ordinary observation backpressure. The response remains a protocol
    /// observation semantically, but command completion depends on admitting
    /// it, so transport must draw from its essential reserve.
    pub(crate) fn with_terminal_response_admission(mut self) -> Self {
        self.admission = RendererOutputPublicationAdmission::EssentialTerminalResponse;
        self
    }

    pub(crate) fn requires_essential_transport_admission(&self) -> bool {
        self.admission != RendererOutputPublicationAdmission::Observation
    }

    pub fn cursor(&self) -> RendererOutputCursor {
        self.cursor
    }

    pub fn ordering(&self) -> RendererOutputPublicationOrdering {
        self.ordering
    }

    pub fn records(&self) -> &[RendererOutputRecord] {
        &self.records
    }

    pub fn into_records(self) -> Vec<RendererOutputRecord> {
        self.records
    }

    pub(crate) fn transport_charge_bytes(&self) -> usize {
        let records = self.records.iter().fold(0usize, |total, record| {
            total.saturating_add(record.transport_charge_bytes())
        });
        std::mem::size_of::<Self>()
            .saturating_add(records)
            .max(4 * 1024)
    }

    #[doc(hidden)]
    pub fn new_for_test(cursor: RendererOutputCursor, records: Vec<RendererOutputRecord>) -> Self {
        Self::new(cursor, records)
    }

    /// Constructs a publication with an explicit scheduler-ordering fact for
    /// cross-crate integration tests.
    ///
    /// Production code derives this fact at the Page turn boundary through
    /// [`Self::with_ordering`]. Exposing only this test constructor keeps
    /// callers from manufacturing post-load ordering after publication.
    #[doc(hidden)]
    pub fn new_for_test_with_ordering(
        cursor: RendererOutputCursor,
        ordering: RendererOutputPublicationOrdering,
        records: Vec<RendererOutputRecord>,
    ) -> Self {
        Self::new(cursor, records).with_ordering(ordering)
    }

    pub(crate) fn publish_to(
        self,
        sender: &RendererOutputTransportSender,
    ) -> Result<(), RendererOutputTransportSendError> {
        sender.send(self.into())
    }
}

#[cfg(test)]
mod tests;

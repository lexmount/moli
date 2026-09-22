use std::{num::NonZeroU64, sync::Arc};

use parking_lot::Mutex;

use super::{
    PendingRendererOutputRecord, RendererOutputCursor, RendererOutputFence,
    RendererOutputPublication, RendererOutputPublicationOrdering, RendererOutputRecord,
    RendererOutputStreamCloseReason, RendererOutputStreamControl, RendererOutputStreamIdentity,
    RendererOutputTransportSender,
};

#[derive(Debug)]
struct RendererTurnOutputJournalState {
    stream: RendererOutputStreamIdentity,
    next_sequence: NonZeroU64,
    last_published_sequence: Option<NonZeroU64>,
    records: Vec<PendingRendererOutputRecord>,
    transport: Option<RendererOutputTransportSender>,
    transport_first_sequence: NonZeroU64,
    closed: bool,
    deferred_publications: Vec<RendererOutputPublication>,
    deferred_close: Option<RendererOutputStreamControl>,
}

#[cfg(test)]
mod retention_tests {
    use super::*;
    use crate::runtime::{
        RendererBrowserContextRuntimeId, RendererProtocolObservation,
        RendererRuntimeInspectorMessage, RendererRuntimeInspectorMessageBatch,
    };

    fn record() -> PendingRendererOutputRecord {
        PendingRendererOutputRecord::observation(
            None,
            RendererProtocolObservation::RuntimeInspector(
                RendererRuntimeInspectorMessageBatch::new(
                    moli_page_types::RendererDevToolsAgentToken::allocate(),
                    moli_page_types::DevToolsSessionKey::Primary,
                    vec![RendererRuntimeInspectorMessage::from_v8_inspector_message(
                        serde_json::json!({"method":"Runtime.consoleAPICalled", "params":{"payload":"x".repeat(16384)}}),
                    )],
                ),
            ),
        )
    }

    #[test]
    fn unobserved_worker_journal_does_not_retain_payloads_or_reserve_history() {
        let journal =
            RendererTurnOutputJournal::new(RendererOutputStreamIdentity::new_shared_worker(
                RendererBrowserContextRuntimeId::new_for_testing(37),
                1,
            ));
        for _ in 0..4096 {
            journal.publish_record(record().resolve().unwrap());
        }
        assert!(
            journal.state.lock().deferred_publications.is_empty(),
            "an unobserved Worker must not retain historical payloads"
        );
        assert!(
            journal.last_published_cursor().is_none(),
            "the observed FIFO starts at binding"
        );
        let (sender, mut receiver) = super::super::renderer_output_transport_channel();
        journal.bind_transport(sender);
        assert!(matches!(
            receiver.try_recv().unwrap(),
            super::super::RendererOutputTransportMessage::StreamControl(
                RendererOutputStreamControl::Opened { .. }
            )
        ));
        assert!(
            receiver.try_recv().is_err(),
            "binding restores native state, not unbounded history"
        );
        journal.publish_record(record().resolve().unwrap());
        let super::super::RendererOutputTransportMessage::Publication(publication) =
            receiver.try_recv().unwrap()
        else {
            panic!("bound Worker output must enter the FIFO")
        };
        assert_eq!(publication.cursor().sequence(), 1);
    }

    #[test]
    fn failed_transport_binding_releases_all_frozen_page_payloads() {
        let journal = RendererTurnOutputJournal::new(
            RendererOutputStreamIdentity::new_page_for_protocol_test(
                crate::runtime::PageId::new_for_testing(8),
            ),
        );
        for _ in 0..128 {
            journal.append(record());
            journal.publish_pending();
        }
        let (sender, receiver) = super::super::renderer_output_transport_channel();
        drop(receiver);
        journal.bind_transport(sender.clone());
        assert!(
            journal.state.lock().deferred_publications.is_empty(),
            "terminal binding cannot retain an unreplayable prefix"
        );
        journal.bind_transport(sender);
        journal.retire(RendererOutputStreamCloseReason::ResidenceRetired);
        assert!(journal.state.lock().deferred_publications.is_empty());
    }
}

/// Shared journal for one exact renderer output stream.
///
/// Page producers append on their owner lane and settle once per selected
/// turn. Worker producers may run on worker/service threads, so stream
/// sequencing and channel admission share this mutex. Holding the lock through
/// `send()` is intentional: two concurrent worker facts must not allocate
/// sequence 1/2 and then enter the transport in 2/1 order.
#[derive(Clone, Debug)]
pub(crate) struct RendererTurnOutputJournal {
    state: Arc<Mutex<RendererTurnOutputJournalState>>,
}

/// Frozen Page output retains its exact producer journal until admission.
/// The transport payload itself never retains the journal.
pub(crate) struct RendererSettledOutput {
    journal: RendererTurnOutputJournal,
    publication: RendererOutputPublication,
}

impl RendererSettledOutput {
    pub(crate) fn new(
        journal: RendererTurnOutputJournal,
        publication: RendererOutputPublication,
    ) -> Self {
        assert_eq!(journal.stream(), publication.cursor().stream());
        Self {
            journal,
            publication,
        }
    }

    pub(crate) fn cursor(&self) -> RendererOutputCursor {
        self.publication.cursor()
    }

    pub(crate) fn with_ordering(mut self, ordering: RendererOutputPublicationOrdering) -> Self {
        self.publication = self.publication.with_ordering(ordering);
        self
    }

    pub(crate) fn publish(self) {
        let mut state = self.journal.state.lock();
        RendererTurnOutputJournal::publish_or_defer_locked(&mut state, self.publication);
    }
}

/// Move-owned records reserved at one exact stream sequence but not yet
/// published. Realm enrichment runs on this value after the journal mutex is
/// released, so an Inspector callback can never re-enter the same journal
/// while its state lock is held.
pub(crate) struct PendingRendererOutputPublication {
    cursor: RendererOutputCursor,
    records: Vec<PendingRendererOutputRecord>,
}

impl PendingRendererOutputPublication {
    pub(crate) fn records_mut(&mut self) -> &mut [PendingRendererOutputRecord] {
        &mut self.records
    }

    pub(crate) fn finish(self) -> RendererOutputPublication {
        let records = self
            .records
            .into_iter()
            .map(|record| {
                record.resolve().unwrap_or_else(|_| {
                    panic!("renderer output publication cannot retain unresolved realm identities")
                })
            })
            .collect();
        RendererOutputPublication::new(self.cursor, records)
    }
}

impl RendererTurnOutputJournal {
    pub(crate) fn new(stream: RendererOutputStreamIdentity) -> Self {
        Self {
            state: Arc::new(Mutex::new(RendererTurnOutputJournalState {
                stream,
                next_sequence: NonZeroU64::MIN,
                last_published_sequence: None,
                records: Vec::new(),
                transport: None,
                transport_first_sequence: NonZeroU64::MIN,
                closed: false,
                deferred_publications: Vec::new(),
                deferred_close: None,
            })),
        }
    }

    pub(crate) fn new_with_transport(
        stream: RendererOutputStreamIdentity,
        transport: RendererOutputTransportSender,
    ) -> Self {
        let journal = Self::new(stream);
        journal.bind_transport(transport);
        journal
    }

    pub(crate) fn stream(&self) -> RendererOutputStreamIdentity {
        self.state.lock().stream
    }

    pub(crate) fn last_published_cursor(&self) -> Option<RendererOutputCursor> {
        let state = self.state.lock();
        state
            .last_published_sequence
            .map(|sequence| RendererOutputCursor::new(state.stream, sequence))
    }

    /// Exports one already-published cursor to an independent completion
    /// channel while retaining the stream's protocol-side retirement state.
    pub(crate) fn declare_fence(&self, cursor: RendererOutputCursor) -> RendererOutputFence {
        let state = self.state.lock();
        assert!(
            !state.closed,
            "renderer output fence must be declared before stream closure"
        );
        assert_eq!(
            cursor.stream(),
            state.stream,
            "renderer output fence must belong to its declaring journal"
        );
        assert!(
            state
                .last_published_sequence
                .is_some_and(|sequence| sequence.get() >= cursor.sequence()),
            "renderer output fence cannot name an unpublished cursor"
        );
        // Declaration occurs while holding the same stream lock used by
        // publication and Close, so the transport observes a deterministic
        // publication -> declaration -> close order.
        let transport = (cursor.sequence() >= state.transport_first_sequence.get())
            .then(|| state.transport.clone())
            .flatten();
        RendererOutputFence::declare(cursor, transport)
    }

    pub(crate) fn append(&self, record: PendingRendererOutputRecord) {
        let mut state = self.state.lock();
        assert!(
            !state.closed,
            "renderer output cannot be appended after stream closure"
        );
        state.records.push(record);
    }

    pub(crate) fn append_records(
        &self,
        records: impl IntoIterator<Item = PendingRendererOutputRecord>,
    ) {
        let mut state = self.state.lock();
        assert!(
            !state.closed,
            "renderer output cannot be appended after stream closure"
        );
        state.records.extend(records);
    }

    /// Appends one non-empty owner-command batch and atomically captures the
    /// cursor of the publication that must contain it.
    pub(crate) fn append_command_records(
        &self,
        records: Vec<PendingRendererOutputRecord>,
    ) -> RendererOutputCursor {
        assert!(
            !records.is_empty(),
            "renderer command output cursor requires at least one record"
        );
        let mut state = self.state.lock();
        assert!(
            !state.closed,
            "renderer output cannot be appended after stream closure"
        );
        let expected_cursor = RendererOutputCursor::new(state.stream, state.next_sequence);
        state.records.extend(records);
        expected_cursor
    }

    #[cfg(test)]
    pub(crate) fn settle(&self) -> Option<RendererOutputPublication> {
        let mut state = self.state.lock();
        Self::settle_locked(&mut state)
    }

    pub(crate) fn take_pending_for_resolution(&self) -> Option<PendingRendererOutputPublication> {
        let mut state = self.state.lock();
        Self::take_pending_locked(&mut state)
    }

    /// Freezes and admits the records produced so far without ending the
    /// enclosing Page turn.
    ///
    /// Most Page output is published when its owner turn returns. A modal
    /// JavaScript dialog and a V8 debugger pause deliberately suspend that
    /// return while protocol must remain able to observe and resolve the
    /// suspension. Those two production boundaries flush their exact prefix
    /// through this method; later records in the same physical turn continue
    /// at the next stream sequence.
    pub(crate) fn publish_pending(&self) -> Option<RendererOutputCursor> {
        let mut state = self.state.lock();
        let publication = Self::settle_locked(&mut state)?;
        let cursor = publication.cursor();
        Self::publish_or_defer_locked(&mut state, publication);
        Some(cursor)
    }

    fn publish_or_defer_locked(
        state: &mut RendererTurnOutputJournalState,
        publication: RendererOutputPublication,
    ) {
        // Resolution can finish after an observer replacement. Its reserved
        // cursor still belongs to the previous observer, not the new FIFO.
        if publication.cursor().sequence() < state.transport_first_sequence.get() {
            return;
        }
        if let Some(transport) = state.transport.as_ref() {
            // A closed transport means the protocol owner has already
            // retired. The concrete prefix is still settled at `cursor`; it
            // must not be put back into the journal and rediscovered by a
            // later owner turn.
            let _ = publication.publish_to(transport);
        } else {
            state.deferred_publications.push(publication);
        }
    }

    /// Atomically appends and publishes one already-resolved producer batch.
    ///
    /// A DevTools session response is a terminal renderer observation, not a
    /// command-side acknowledgement. Admitting it through the same journal as
    /// that session's notifications keeps their producer order without a
    /// second completion sequencer. The returned fence names the terminal
    /// batch itself so the command completion cannot release post-response
    /// owner actions before protocol ingress admits that batch. `None` means
    /// the exact attachment stream was closed, unbound, or rejected the
    /// publication before it could take ownership of the records.
    pub(crate) fn try_publish_terminal_records_and_declare_fence(
        &self,
        records: impl IntoIterator<Item = PendingRendererOutputRecord>,
    ) -> Option<RendererOutputFence> {
        let records = records.into_iter().collect::<Vec<_>>();
        assert!(
            !records.is_empty(),
            "renderer output publication must contain at least one record"
        );
        let mut state = self.state.lock();
        if state.closed {
            return None;
        }
        // A terminal response cannot settle against a deferred publication:
        // the DevTools session has no adapter-reply fallback if later
        // transport admission fails.
        let transport = state.transport.clone()?;
        state.records.extend(records);
        let publication = Self::settle_locked(&mut state)
            .expect("newly appended renderer output records must settle")
            .with_terminal_response_admission();
        let cursor = publication.cursor();
        // Once this exact publication is rejected, the bounded transport is
        // terminal. Report the rejection to the response authority instead
        // of manufacturing a fence and falsely settling the frontend call.
        if publication.publish_to(&transport).is_err() {
            return None;
        }
        Some(RendererOutputFence::declare(cursor, Some(transport)))
    }

    fn settle_locked(
        state: &mut RendererTurnOutputJournalState,
    ) -> Option<RendererOutputPublication> {
        Self::take_pending_locked(state).map(PendingRendererOutputPublication::finish)
    }

    fn take_pending_locked(
        state: &mut RendererTurnOutputJournalState,
    ) -> Option<PendingRendererOutputPublication> {
        assert!(
            !state.closed,
            "renderer output stream cannot settle after closure"
        );
        if state.records.is_empty() {
            return None;
        }
        let cursor = Self::reserve_cursor_locked(state);
        let records = std::mem::take(&mut state.records);
        Some(PendingRendererOutputPublication { cursor, records })
    }

    fn reserve_cursor_locked(state: &mut RendererTurnOutputJournalState) -> RendererOutputCursor {
        let sequence = state.next_sequence;
        state.next_sequence = NonZeroU64::new(
            sequence
                .get()
                .checked_add(1)
                .expect("renderer output stream sequence exhausted"),
        )
        .expect("renderer output stream sequence wrapped to zero");
        state.last_published_sequence = Some(sequence);
        RendererOutputCursor::new(state.stream, sequence)
    }

    /// Publishes one already-resolved Worker fact at its production boundary.
    ///
    /// Page turns use [`Self::append`] plus [`Self::settle`]; Worker streams
    /// have no enclosing Page turn, so this operation is their atomic
    /// append/settle/admit boundary.
    pub(crate) fn publish_record(&self, record: RendererOutputRecord) {
        let mut state = self.state.lock();
        assert!(
            !state.closed,
            "renderer output cannot be published after stream closure"
        );
        if state.transport.is_none()
            && !matches!(
                state.stream.residence(),
                super::RendererOutputResidenceIdentity::Page { .. }
            )
        {
            // Native Worker state is already reported to Browser. No frontend
            // owns this history; its FIFO begins when observation is bound.
            return;
        }
        let cursor = Self::reserve_cursor_locked(&mut state);
        let publication = RendererOutputPublication::new(cursor, vec![record]);
        Self::publish_or_defer_locked(&mut state, publication);
    }

    /// Binds an observer, replaying the initial frozen prefix only on first
    /// binding. A closed observer may be replaced while the residence lives.
    pub(crate) fn bind_transport(&self, transport: RendererOutputTransportSender) {
        let mut state = self.state.lock();
        if let Some(existing) = state.transport.as_ref() {
            if existing.same_channel(&transport) {
                return;
            }
            assert!(
                existing.is_closed(),
                "renderer output cannot replace a live observer"
            );
            if state.closed {
                return;
            }
            state.transport_first_sequence = state.next_sequence;
        }
        // Record binding before the first send: rejection consumes the frozen
        // prefix, so a later observer can never replay it.
        state.transport = Some(transport.clone());
        let deferred = std::mem::take(&mut state.deferred_publications);
        let close = state.deferred_close.take();
        let opened = RendererOutputStreamControl::Opened {
            stream: state.stream,
            first_sequence: state.transport_first_sequence,
        };
        if transport.send(opened.into()).is_err() {
            return;
        }
        for publication in deferred {
            if publication.publish_to(&transport).is_err() {
                return;
            }
        }
        // Unsettled records still belong to an active producer turn. A late
        // observer may replay frozen publications, but must not resolve or
        // publish the producer's in-progress records from another lane.
        if let Some(control) = close {
            let _ = transport.send(control.into());
        }
    }

    fn close_locked(
        state: &mut RendererTurnOutputJournalState,
        reason: RendererOutputStreamCloseReason,
    ) -> RendererOutputStreamControl {
        assert!(
            state.records.is_empty(),
            "renderer output stream cannot close with unsettled records"
        );
        assert!(!state.closed, "renderer output stream closed twice");
        state.closed = true;
        RendererOutputStreamControl::Closed {
            stream: state.stream,
            last_published_sequence: state.last_published_sequence,
            reason,
        }
    }

    pub(crate) fn retire(&self, reason: RendererOutputStreamCloseReason) {
        // Context teardown is itself the final owner-lane turn and may append
        // a terminal lifecycle record. Freeze that record before closing the
        // stream so protocol ingress always observes:
        //
        //     final publication -> Closed(last_published_sequence)
        //
        // `settle()` also rejects unresolved records, preventing teardown from
        // laundering an unfinished producer turn into a valid stream close.
        let mut state = self.state.lock();
        let final_publication = Self::settle_locked(&mut state);
        let control = Self::close_locked(&mut state, reason);
        if let Some(transport) = state.transport.as_ref() {
            if let Some(publication) = final_publication {
                let _ = publication.publish_to(transport);
            }
            // A closed channel means the protocol attachment/runtime already
            // reached its terminal boundary. Page teardown must still retire
            // its local stream state without turning normal shutdown into a
            // process-fatal invariant.
            let _ = transport.send(control.into());
        } else {
            // Page owner actions retain their final concrete turn and close
            // boundary. Unobserved Worker journals have no historical records
            // and their registry releases them at retirement.
            if let Some(publication) = final_publication {
                state.deferred_publications.push(publication);
            }
            state.deferred_close = Some(control);
        }
    }

    #[cfg(test)]
    pub(crate) fn pending_len(&self) -> usize {
        self.state.lock().records.len()
    }
}

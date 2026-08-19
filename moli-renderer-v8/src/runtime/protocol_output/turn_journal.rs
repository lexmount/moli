use std::{num::NonZeroU64, sync::Arc};

use parking_lot::Mutex;

use super::{
    PendingRendererOutputRecord, RendererOutputCursor, RendererOutputFence,
    RendererOutputPublication, RendererOutputRecord, RendererOutputStreamCloseReason,
    RendererOutputStreamControl, RendererOutputStreamIdentity, RendererOutputTransportSender,
};

#[derive(Debug)]
struct RendererTurnOutputJournalState {
    stream: RendererOutputStreamIdentity,
    next_sequence: NonZeroU64,
    last_published_sequence: Option<NonZeroU64>,
    records: Vec<PendingRendererOutputRecord>,
    transport: Option<RendererOutputTransportSender>,
    closed: bool,
    deferred_publications: Vec<RendererOutputPublication>,
    deferred_close: Option<RendererOutputStreamControl>,
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
        RendererOutputFence::declare(cursor, state.transport.clone())
    }

    pub(crate) fn transport_is_bound(&self) -> bool {
        self.state.lock().transport.is_some()
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
        if let Some(transport) = state.transport.as_ref() {
            // A closed transport means the protocol owner has already
            // retired. The concrete prefix is still settled at `cursor`; it
            // must not be put back into the journal and rediscovered by a
            // later owner turn.
            let _ = publication.publish_to(transport);
        } else {
            state.deferred_publications.push(publication);
        }
        Some(cursor)
    }

    /// Atomically appends and publishes one already-resolved producer batch.
    ///
    /// A DevTools session response is a terminal renderer observation, not a
    /// command-side acknowledgement. Admitting it through the same journal as
    /// that session's notifications keeps their producer order without a
    /// second completion sequencer. `false` means the exact attachment stream
    /// was already closed before it could take ownership of the records.
    pub(crate) fn try_publish_records(
        &self,
        records: impl IntoIterator<Item = PendingRendererOutputRecord>,
    ) -> bool {
        let records = records.into_iter().collect::<Vec<_>>();
        assert!(
            !records.is_empty(),
            "renderer output publication must contain at least one record"
        );
        let mut state = self.state.lock();
        if state.closed {
            return false;
        }
        state.records.extend(records);
        let publication = Self::settle_locked(&mut state)
            .expect("newly appended renderer output records must settle");
        if let Some(transport) = state.transport.as_ref() {
            // Admission failure means the protocol owner has retired or its
            // bounded stream is terminal. The journal has nevertheless taken
            // ownership at this exact sequence; falling back to another sink
            // here could duplicate the response.
            let _ = publication.publish_to(transport);
        } else {
            state.deferred_publications.push(publication);
        }
        true
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
        let cursor = Self::reserve_cursor_locked(&mut state);
        let publication = RendererOutputPublication::new(cursor, vec![record]);
        if let Some(transport) = state.transport.as_ref() {
            // A closed channel means the protocol owner has already retired.
            // The renderer stream remains settled and must not resurrect the
            // record on a later transport.
            let _ = publication.publish_to(transport);
        } else {
            // Keep the already sequenced concrete batch frozen until the
            // browser context binds its protocol transport.
            state.deferred_publications.push(publication);
        }
    }

    /// Binds the browser-context protocol transport to a stream that may have
    /// produced facts before a CDP connection installed its receiver.
    pub(crate) fn bind_transport(&self, transport: RendererOutputTransportSender) {
        let mut state = self.state.lock();
        if let Some(existing) = state.transport.as_ref() {
            assert!(
                existing.same_channel(&transport),
                "renderer output stream cannot be rebound to a different transport"
            );
            return;
        }
        // Binding is a one-shot stream-lifecycle transition, including for a
        // stream which already retired before protocol transport existed.
        // Record it before the first send: a closed receiver is a terminal
        // protocol boundary, not permission to replay Opened or a concrete
        // prefix into a later, unrelated transport.
        state.transport = Some(transport.clone());
        let opened = RendererOutputStreamControl::Opened {
            stream: state.stream,
        };
        if transport.send(opened.into()).is_err() {
            return;
        }
        for publication in &state.deferred_publications {
            if publication.clone().publish_to(&transport).is_err() {
                return;
            }
        }
        state.deferred_publications.clear();
        if !state.records.is_empty() {
            let publication = Self::settle_locked(&mut state)
                .expect("pre-transport renderer records must settle");
            if publication.publish_to(&transport).is_err() {
                return;
            }
        }
        if let Some(control) = state.deferred_close.take() {
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
            // A stream may finish before protocol installs a transport (for
            // example, a short-lived worker created during connection setup).
            // Preserve the final concrete fact and close boundary for ordered
            // late transport binding; do not synthesize them from live state.
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

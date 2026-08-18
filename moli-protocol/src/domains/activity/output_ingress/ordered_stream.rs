use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use moli_core::{
    PageId, RendererOutputCursor, RendererOutputFenceLeaseId, RendererOutputPublication,
    RendererOutputResidenceIdentity, RendererOutputStreamIdentity, RendererOwnerLocalHostId,
};

use super::super::publication_route::RendererPublicationOwner;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AdmittedRendererOutputPublication {
    publication: RendererOutputPublication,
    owner: RendererPublicationOwner,
}

impl AdmittedRendererOutputPublication {
    fn new(publication: RendererOutputPublication, owner: RendererPublicationOwner) -> Self {
        Self { publication, owner }
    }

    pub(crate) fn into_parts(self) -> (RendererOutputPublication, RendererPublicationOwner) {
        (self.publication, self.owner)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RendererOutputIngressAdmission {
    Ready(Vec<AdmittedRendererOutputPublication>),
    Buffered,
    Stale,
}

#[derive(Debug)]
struct OpenRendererOutputStream {
    owner: Option<RendererPublicationOwner>,
    next_expected_sequence: u64,
    last_projected_sequence: u64,
    projecting_sequence: Option<u64>,
    pending: BTreeMap<u64, RendererOutputPublication>,
    /// `None` means the stream is open. `Some(None)` is a closing stream
    /// which never published a batch.
    closing_at: Option<Option<u64>>,
    /// Exact cursors exported over an independent command/result channel.
    /// These are lifetime capabilities, not additional FIFO predecessors.
    active_cursor_leases: HashMap<RendererOutputFenceLeaseId, u64>,
}

#[derive(Debug)]
struct ClosedRendererOutputStream {
    last_projected_sequence: Option<u64>,
    active_cursor_leases: HashSet<RendererOutputFenceLeaseId>,
}

/// Connection-owned ordering authority for concrete renderer output.
///
/// Channel arrival order is not used as an implicit cross-residence order.
/// Each renderer residence owns an explicitly opened stream. Publications
/// retain their original cursor while waiting for a missing same-stream
/// sequence. Different target/worker streams are independent protocol order
/// domains and are never joined by a command response.
#[derive(Default)]
pub(crate) struct OrderedRendererOutputIngress {
    open_streams: HashMap<RendererOutputStreamIdentity, OpenRendererOutputStream>,
    closed_streams: HashMap<RendererOutputStreamIdentity, ClosedRendererOutputStream>,
    pending_owners: HashMap<RendererOutputResidenceIdentity, RendererPublicationOwner>,
    /// Stable readiness order only chooses between independent streams. A
    /// stream appears at most once, so a missing same-stream predecessor does
    /// not make every later admission rescan all buffered publications.
    ready_streams: VecDeque<RendererOutputStreamIdentity>,
    queued_ready_streams: HashSet<RendererOutputStreamIdentity>,
}

impl OrderedRendererOutputIngress {
    pub(crate) fn open(
        &mut self,
        stream: RendererOutputStreamIdentity,
        discovered_owner: Option<RendererPublicationOwner>,
    ) {
        assert!(
            !self.open_streams.contains_key(&stream) && !self.closed_streams.contains_key(&stream),
            "renderer output stream opened more than once"
        );
        let registered_owner = self.pending_owners.remove(&stream.residence());
        if let (Some(registered), Some(discovered)) = (&registered_owner, &discovered_owner) {
            assert_eq!(
                registered, discovered,
                "registered and discovered renderer output owners must match"
            );
        }
        self.open_streams.insert(
            stream,
            OpenRendererOutputStream {
                owner: registered_owner.or(discovered_owner),
                next_expected_sequence: 1,
                last_projected_sequence: 0,
                projecting_sequence: None,
                pending: BTreeMap::new(),
                closing_at: None,
                active_cursor_leases: HashMap::new(),
            },
        );
    }

    /// Retains the completed projection frontier while an exact cursor can
    /// still arrive through a command response or navigation boundary.
    ///
    /// The declaration itself is sent after the named publication and before
    /// a possible `Closed`. Scheduler load ordering may park the publication
    /// while processing the later declaration, so admission is deliberately
    /// not required here; `Closed` validates every declared sequence against
    /// the authoritative renderer tail.
    pub(crate) fn declare_cursor_lease(
        &mut self,
        cursor: RendererOutputCursor,
        lease_id: RendererOutputFenceLeaseId,
    ) {
        let state = self
            .open_streams
            .get_mut(&cursor.stream())
            .expect("renderer output cursor lease declared outside its open stream lifetime");
        assert!(
            state
                .active_cursor_leases
                .insert(lease_id, cursor.sequence())
                .is_none(),
            "renderer output cursor lease declared more than once"
        );
    }

    pub(crate) fn release_cursor_lease(
        &mut self,
        stream: RendererOutputStreamIdentity,
        lease_id: RendererOutputFenceLeaseId,
    ) {
        if let Some(state) = self.open_streams.get_mut(&stream) {
            assert!(
                state.active_cursor_leases.remove(&lease_id).is_some(),
                "unknown renderer output cursor lease released from an open stream"
            );
            return;
        }

        let remove_tombstone = {
            let state = self
                .closed_streams
                .get_mut(&stream)
                .expect("renderer output cursor lease released after its stream was reclaimed");
            assert!(
                state.active_cursor_leases.remove(&lease_id),
                "unknown renderer output cursor lease released from a closed stream"
            );
            state.active_cursor_leases.is_empty()
        };
        if remove_tombstone {
            self.closed_streams.remove(&stream);
        }
    }

    /// Binds a renderer residence before its first concrete publication.
    ///
    /// Navigation can construct the renderer agent (and therefore emit
    /// `Opened`) before protocol commit binds that Page reservation to a
    /// target. Conversely, initial-document construction binds the reservation
    /// first. Supporting both orders here keeps transport scheduling from
    /// becoming an ownership contract.
    pub(crate) fn bind_owner(
        &mut self,
        residence: RendererOutputResidenceIdentity,
        owner: RendererPublicationOwner,
    ) {
        let mut matched_open_stream = false;
        for (stream, state) in &mut self.open_streams {
            if stream.residence() != residence {
                continue;
            }
            matched_open_stream = true;
            assert!(
                state.pending.is_empty(),
                "renderer output owner must bind before the first publication"
            );
            if let Some(existing) = &state.owner {
                assert_eq!(
                    existing, &owner,
                    "one renderer output stream cannot change protocol owner"
                );
            } else {
                state.owner = Some(owner.clone());
            }
        }
        if matched_open_stream {
            return;
        }
        if let Some(existing) = self.pending_owners.insert(residence, owner.clone()) {
            assert_eq!(
                existing, owner,
                "one renderer residence cannot register two protocol owners"
            );
        }
    }

    pub(crate) fn release_page_owner_reservation(
        &mut self,
        owner_local_host_id: RendererOwnerLocalHostId,
        page_id: PageId,
    ) {
        self.pending_owners
            .remove(&RendererOutputResidenceIdentity::Page {
                owner_local_host_id,
                page_id,
            });
    }

    pub(crate) fn close(
        &mut self,
        stream: RendererOutputStreamIdentity,
        last_published_sequence: Option<std::num::NonZeroU64>,
    ) {
        let state = self
            .open_streams
            .get_mut(&stream)
            .expect("renderer output stream closed before it was opened");
        assert!(
            state.closing_at.is_none(),
            "renderer output stream closed more than once"
        );
        let last_published_sequence = last_published_sequence.map(|sequence| sequence.get());
        let admitted_tail = state
            .next_expected_sequence
            .checked_sub(1)
            .expect("renderer output stream next sequence must remain non-zero");
        assert!(
            last_published_sequence.is_some_and(|last| last >= admitted_tail)
                || (last_published_sequence.is_none() && admitted_tail == 0),
            "renderer output stream closed before its admitted cursor"
        );
        assert!(
            state
                .pending
                .last_key_value()
                .is_none_or(|(sequence, _)| Some(*sequence) <= last_published_sequence),
            "renderer output stream closed before a pending publication"
        );
        assert!(
            state
                .active_cursor_leases
                .values()
                .all(|sequence| { last_published_sequence.is_some_and(|last| *sequence <= last) }),
            "renderer output cursor lease names a publication beyond the declared stream tail"
        );
        state.closing_at = Some(last_published_sequence);
        self.retire_closed_stream_if_complete(stream);
    }

    pub(crate) fn admit(
        &mut self,
        publication: RendererOutputPublication,
    ) -> RendererOutputIngressAdmission {
        let cursor = publication.cursor();
        if self.closed_streams.contains_key(&cursor.stream()) {
            return RendererOutputIngressAdmission::Stale;
        }
        if !self.open_streams.contains_key(&cursor.stream()) {
            panic!(
                "renderer output publication {cursor:?} arrived before stream open; \
                 open streams={:?}, closed streams={:?}, pending owner residences={:?}",
                self.open_streams.keys().collect::<Vec<_>>(),
                self.closed_streams.keys().collect::<Vec<_>>(),
                self.pending_owners.keys().collect::<Vec<_>>(),
            );
        }
        let state = self
            .open_streams
            .get_mut(&cursor.stream())
            .expect("renderer output stream existence checked above");
        assert!(
            state.owner.is_some(),
            "renderer output publication {cursor:?} arrived before owner binding for residence {:?}",
            cursor.stream().residence(),
        );
        if cursor.sequence() < state.next_expected_sequence
            || state.pending.contains_key(&cursor.sequence())
        {
            return RendererOutputIngressAdmission::Stale;
        }
        if let Some(closing_at) = state.closing_at
            && closing_at.is_none_or(|last| cursor.sequence() > last)
        {
            return RendererOutputIngressAdmission::Stale;
        }
        state.pending.insert(cursor.sequence(), publication);
        self.enqueue_stream_if_ready(cursor.stream());

        let ready = self.release_ready_publications();
        if ready.is_empty() {
            RendererOutputIngressAdmission::Buffered
        } else {
            RendererOutputIngressAdmission::Ready(ready)
        }
    }

    /// Marks one already-admitted publication fully consumed by its protocol
    /// owner.
    ///
    /// Admission only removes a publication from the transport FIFO. Its
    /// typed projections may still be awaiting an owner action such as popup
    /// target creation. Runtime responses fence on projection completion, not
    /// admission, so a WindowProxy can never escape before its browsing
    /// context exists.
    pub(crate) fn complete_projection(
        &mut self,
        cursor: RendererOutputCursor,
    ) -> Vec<AdmittedRendererOutputPublication> {
        {
            let state = self
                .open_streams
                .get_mut(&cursor.stream())
                .expect("renderer output projection completed after stream retirement");
            assert_eq!(
                state.projecting_sequence,
                Some(cursor.sequence()),
                "renderer output projection must complete its exact same-stream lease"
            );
            let expected = state
                .last_projected_sequence
                .checked_add(1)
                .expect("renderer output projected sequence exhausted");
            assert_eq!(
                cursor.sequence(),
                expected,
                "renderer output projections must complete in same-stream FIFO order"
            );
            state.projecting_sequence = None;
            state.last_projected_sequence = cursor.sequence();
        }
        self.enqueue_stream_if_ready(cursor.stream());
        let ready = self.release_ready_publications();
        self.retire_closed_stream_if_complete(cursor.stream());
        ready
    }

    /// Returns whether the exact concrete cursor has finished every typed
    /// projection and browser-owner action at the protocol boundary.
    ///
    /// A closed stream may satisfy the cursor only when the cursor was part of
    /// its declared tail; naming a later cursor is an internal ownership
    /// error, not a condition that can become true by waiting.
    pub(crate) fn is_projection_complete(&self, cursor: RendererOutputCursor) -> bool {
        if let Some(state) = self.open_streams.get(&cursor.stream()) {
            return cursor.sequence() <= state.last_projected_sequence;
        }
        if let Some(state) = self.closed_streams.get(&cursor.stream()) {
            assert!(
                state
                    .last_projected_sequence
                    .is_some_and(|last| cursor.sequence() <= last),
                "renderer output publication names a predecessor beyond a closed stream tail"
            );
            return true;
        }
        false
    }

    fn release_ready_publications(&mut self) -> Vec<AdmittedRendererOutputPublication> {
        let mut ready = Vec::new();
        while let Some(stream) = self.ready_streams.pop_front() {
            self.queued_ready_streams.remove(&stream);
            let Some(state) = self.open_streams.get_mut(&stream) else {
                continue;
            };
            if state.projecting_sequence.is_some() {
                continue;
            }
            let sequence = state.next_expected_sequence;
            let Some(publication) = state.pending.remove(&sequence) else {
                continue;
            };
            let owner = state
                .owner
                .clone()
                .expect("renderer output publication arrived before owner binding");
            state.next_expected_sequence = state
                .next_expected_sequence
                .checked_add(1)
                .expect("renderer output ingress sequence exhausted");
            state.projecting_sequence = Some(sequence);
            ready.push(AdmittedRendererOutputPublication::new(publication, owner));
        }
        ready
    }

    fn enqueue_stream_if_ready(&mut self, stream: RendererOutputStreamIdentity) {
        let is_ready = self.open_streams.get(&stream).is_some_and(|state| {
            state.projecting_sequence.is_none()
                && state.pending.contains_key(&state.next_expected_sequence)
        });
        if is_ready && self.queued_ready_streams.insert(stream) {
            self.ready_streams.push_back(stream);
        }
    }

    fn retire_closed_stream_if_complete(&mut self, stream: RendererOutputStreamIdentity) {
        let Some(state) = self.open_streams.get(&stream) else {
            return;
        };
        let Some(last_published_sequence) = state.closing_at else {
            return;
        };
        let admitted_tail = state
            .next_expected_sequence
            .checked_sub(1)
            .expect("renderer output stream next sequence must remain non-zero");
        if !state.pending.is_empty()
            || last_published_sequence != ((admitted_tail != 0).then_some(admitted_tail))
            || state.last_projected_sequence != admitted_tail
            || state.projecting_sequence.is_some()
        {
            return;
        }
        let state = self
            .open_streams
            .remove(&stream)
            .expect("completed renderer output stream must remain open until retirement");
        self.queued_ready_streams.remove(&stream);
        if !state.active_cursor_leases.is_empty() {
            let active_cursor_leases = state.active_cursor_leases.into_keys().collect();
            assert!(
                self.closed_streams
                    .insert(
                        stream,
                        ClosedRendererOutputStream {
                            last_projected_sequence: last_published_sequence,
                            active_cursor_leases,
                        },
                    )
                    .is_none(),
                "renderer output stream retired more than once"
            );
        }
        // A fully projected Page or Worker without an externally owned cursor
        // has no future observer. Reclaim it immediately instead of retaining
        // one tombstone per retired residence.
    }

    #[cfg(test)]
    fn pending_owner_count(&self) -> usize {
        self.pending_owners.len()
    }

    #[cfg(test)]
    fn closed_stream_count(&self) -> usize {
        self.closed_streams.len()
    }
}

#[cfg(test)]
mod tests {
    use moli_core::{
        PageId, RendererOutputItem, RendererOutputRecord, RendererProtocolObservation,
        page::{
            RendererDocumentLifecycleEvent, RendererDocumentLifecycleEventKind,
            RendererDocumentToken, RendererFrameToken, RendererLifecycleEpoch,
            RendererLifecycleStartReason,
        },
    };

    use super::*;

    fn publication(
        stream: RendererOutputStreamIdentity,
        sequence: u64,
    ) -> RendererOutputPublication {
        RendererOutputPublication::new_for_test(
            RendererOutputCursor::new_for_test(stream, sequence),
            vec![RendererOutputRecord::new_for_test(
                RendererOutputItem::Observation(RendererProtocolObservation::DocumentLifecycle(
                    RendererDocumentLifecycleEvent {
                        frame: RendererFrameToken {
                            page_id: PageId::new_for_testing(1),
                        },
                        document: RendererDocumentToken::new_for_testing(
                            PageId::new_for_testing(1),
                            1,
                        ),
                        epoch: RendererLifecycleEpoch(1),
                        sequence,
                        timestamp_micros: sequence,
                        kind: RendererDocumentLifecycleEventKind::Started {
                            reason: RendererLifecycleStartReason::InitialDocument,
                        },
                    },
                )),
            )],
        )
    }

    fn owner() -> RendererPublicationOwner {
        page_owner(PageId::new_for_testing(1))
    }

    fn page_owner(page_id: PageId) -> RendererPublicationOwner {
        let renderer_page = crate::conn::RendererPageResidenceIdentity::new(
            moli_core::RendererOwnerLocalHostId::new_for_testing(1),
            page_id,
        );
        RendererPublicationOwner::PageTarget {
            browser_context_id: "BID-test".to_owned(),
            target_id: Some("TID-test".to_owned()),
            renderer_page,
            page_owner: crate::conn::TargetPageResidenceIdentity::new_for_test(
                "BID-test".to_owned(),
                Some("TID-test".to_owned()),
                1,
            ),
        }
    }

    fn sequences(ready: &[AdmittedRendererOutputPublication]) -> Vec<u64> {
        ready
            .iter()
            .map(|admitted| admitted.publication.cursor().sequence())
            .collect()
    }

    #[test]
    fn stream_requires_open_and_exact_sequence_then_closes_at_observed_tail() {
        let stream =
            RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(7));
        let mut ingress = OrderedRendererOutputIngress::default();
        ingress.open(stream, Some(owner()));
        assert!(matches!(
            ingress.admit(publication(stream, 1)),
            RendererOutputIngressAdmission::Ready(ready) if sequences(&ready) == vec![1]
        ));
        assert!(
            ingress
                .complete_projection(RendererOutputCursor::new_for_test(stream, 1))
                .is_empty()
        );
        assert!(matches!(
            ingress.admit(publication(stream, 2)),
            RendererOutputIngressAdmission::Ready(ready) if sequences(&ready) == vec![2]
        ));
        assert!(
            ingress
                .complete_projection(RendererOutputCursor::new_for_test(stream, 2))
                .is_empty()
        );
        ingress.close(stream, std::num::NonZeroU64::new(2));
        assert_eq!(
            ingress.closed_stream_count(),
            0,
            "a fully projected stream with no exported cursor must be reclaimed immediately"
        );
    }

    #[test]
    fn admission_does_not_satisfy_response_fence_until_projection_completes() {
        let stream =
            RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(17));
        let cursor = RendererOutputCursor::new_for_test(stream, 1);
        let mut ingress = OrderedRendererOutputIngress::default();
        ingress.open(stream, Some(owner()));
        let lease_id = RendererOutputFenceLeaseId::new_for_test(1);
        ingress.declare_cursor_lease(cursor, lease_id);

        assert!(matches!(
            ingress.admit(publication(stream, 1)),
            RendererOutputIngressAdmission::Ready(ready) if sequences(&ready) == vec![1]
        ));
        assert!(
            !ingress.is_projection_complete(cursor),
            "ordered transport admission must not release a response while its owner action is still running"
        );

        // Stream closure can race the asynchronous projection. Retirement
        // must retain the completion state until that final action returns.
        ingress.close(stream, std::num::NonZeroU64::new(1));
        assert!(!ingress.is_projection_complete(cursor));
        assert!(ingress.complete_projection(cursor).is_empty());
        assert!(ingress.is_projection_complete(cursor));
        assert_eq!(ingress.closed_stream_count(), 1);
        ingress.release_cursor_lease(stream, lease_id);
        assert_eq!(
            ingress.closed_stream_count(),
            0,
            "dropping the last exported cursor must reclaim the closed-stream tombstone"
        );
    }

    #[test]
    fn stream_buffers_sequence_gaps_and_releases_the_original_publications_in_order() {
        let stream =
            RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(8));
        let mut ingress = OrderedRendererOutputIngress::default();
        ingress.open(stream, Some(owner()));
        assert_eq!(
            ingress.admit(publication(stream, 2)),
            RendererOutputIngressAdmission::Buffered
        );
        let RendererOutputIngressAdmission::Ready(ready) = ingress.admit(publication(stream, 1))
        else {
            panic!("filling a stream gap must release its concrete head");
        };
        assert_eq!(sequences(&ready), vec![1]);
        let ready = ingress.complete_projection(RendererOutputCursor::new_for_test(stream, 1));
        assert_eq!(sequences(&ready), vec![2]);
        assert!(
            ingress
                .complete_projection(RendererOutputCursor::new_for_test(stream, 2))
                .is_empty()
        );
    }

    #[test]
    fn missing_sequence_queues_each_ready_stream_once_without_blocking_other_streams() {
        let first =
            RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(18));
        let second =
            RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(19));
        let mut ingress = OrderedRendererOutputIngress::default();
        ingress.open(first, Some(owner()));
        ingress.open(second, Some(owner()));

        assert_eq!(
            ingress.admit(publication(first, 2)),
            RendererOutputIngressAdmission::Buffered
        );
        assert!(matches!(
            ingress.admit(publication(second, 1)),
            RendererOutputIngressAdmission::Ready(ready) if sequences(&ready) == vec![1]
        ));
        assert!(matches!(
            ingress.admit(publication(first, 1)),
            RendererOutputIngressAdmission::Ready(ready) if sequences(&ready) == vec![1]
        ));
        assert_eq!(
            sequences(&ingress.complete_projection(RendererOutputCursor::new_for_test(first, 1))),
            vec![2]
        );
    }

    #[test]
    fn close_waits_for_a_declared_but_not_yet_admitted_tail() {
        let stream =
            RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(11));
        let mut ingress = OrderedRendererOutputIngress::default();
        ingress.open(stream, Some(owner()));
        assert_eq!(
            ingress.admit(publication(stream, 2)),
            RendererOutputIngressAdmission::Buffered
        );
        ingress.close(stream, std::num::NonZeroU64::new(2));
        assert_eq!(
            ingress.admit(publication(stream, 3)),
            RendererOutputIngressAdmission::Stale
        );
        assert!(matches!(
            ingress.admit(publication(stream, 1)),
            RendererOutputIngressAdmission::Ready(ready) if sequences(&ready) == vec![1]
        ));
        let ready = ingress.complete_projection(RendererOutputCursor::new_for_test(stream, 1));
        assert_eq!(sequences(&ready), vec![2]);
        assert!(
            ingress
                .complete_projection(RendererOutputCursor::new_for_test(stream, 2))
                .is_empty()
        );
        assert_eq!(ingress.closed_stream_count(), 0);
    }

    #[test]
    fn owner_binding_may_follow_stream_open_before_the_first_publication() {
        let stream =
            RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(12));
        let expected_owner = owner();
        let mut ingress = OrderedRendererOutputIngress::default();
        ingress.open(stream, None);
        ingress.bind_owner(stream.residence(), expected_owner.clone());

        let RendererOutputIngressAdmission::Ready(ready) = ingress.admit(publication(stream, 1))
        else {
            panic!("binding an open stream must admit its first publication");
        };
        let (_, actual_owner) = ready
            .into_iter()
            .next()
            .expect("one publication should be ready")
            .into_parts();
        assert_eq!(actual_owner, expected_owner);
    }

    #[test]
    fn owner_binding_may_precede_stream_open() {
        let stream =
            RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(13));
        let expected_owner = owner();
        let mut ingress = OrderedRendererOutputIngress::default();
        ingress.bind_owner(stream.residence(), expected_owner.clone());
        ingress.open(stream, None);
        let RendererOutputResidenceIdentity::Page {
            owner_local_host_id,
            page_id,
        } = stream.residence()
        else {
            unreachable!("this fixture creates a Page stream")
        };
        ingress.release_page_owner_reservation(owner_local_host_id, page_id);

        let RendererOutputIngressAdmission::Ready(ready) = ingress.admit(publication(stream, 1))
        else {
            panic!("a pre-bound stream must admit its first publication");
        };
        let (_, actual_owner) = ready
            .into_iter()
            .next()
            .expect("one publication should be ready")
            .into_parts();
        assert_eq!(actual_owner, expected_owner);
    }

    #[test]
    fn exact_reservation_release_clears_many_concurrent_page_owners() {
        let mut ingress = OrderedRendererOutputIngress::default();
        let mut reservations = Vec::new();

        for raw_page_id in 20..=275 {
            let page_id = PageId::new_for_testing(raw_page_id);
            let stream = RendererOutputStreamIdentity::new_page_for_protocol_test(page_id);
            ingress.bind_owner(stream.residence(), page_owner(page_id));
            let RendererOutputResidenceIdentity::Page {
                owner_local_host_id,
                ..
            } = stream.residence()
            else {
                unreachable!("this fixture creates Page reservations")
            };
            reservations.push((owner_local_host_id, page_id));
        }
        assert_eq!(
            ingress.pending_owner_count(),
            256,
            "concurrent navigation reservations for one target remain independently addressable"
        );

        for (owner_local_host_id, page_id) in reservations {
            ingress.release_page_owner_reservation(owner_local_host_id, page_id);
        }
        assert_eq!(ingress.pending_owner_count(), 0);
    }

    #[test]
    fn fully_projected_worker_stream_does_not_leave_a_command_fence_tombstone() {
        let mut ingress = OrderedRendererOutputIngress::default();

        for instance_id in 1..=256 {
            let stream =
                RendererOutputStreamIdentity::new_shared_worker_for_protocol_test(instance_id);
            let cursor = RendererOutputCursor::new_for_test(stream, 1);
            ingress.open(
                stream,
                Some(RendererPublicationOwner::BrowserContext {
                    browser_context_id: "BID-test".to_owned(),
                }),
            );
            assert!(matches!(
                ingress.admit(publication(stream, 1)),
                RendererOutputIngressAdmission::Ready(ready) if sequences(&ready) == vec![1]
            ));
            ingress.close(stream, std::num::NonZeroU64::new(1));
            assert!(ingress.complete_projection(cursor).is_empty());
        }
        assert_eq!(ingress.closed_stream_count(), 0);
    }
}
